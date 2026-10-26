package hyper.cdc.jdbcadmin;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;

public final class Main {
    private static final ObjectMapper JSON = new ObjectMapper();

    public static void main(String[] args) throws IOException {
        String host = env("JDBC_ADMIN_HOST", "0.0.0.0");
        int port = Integer.parseInt(env("JDBC_ADMIN_PORT", "8084"));
        String token = firstNonBlank(System.getenv("ORCH_JDBC_ADMIN_TOKEN"), System.getenv("JDBC_ADMIN_TOKEN"));

        HttpServer server = HttpServer.create(new InetSocketAddress(host, port), 0);
        server.createContext("/health", exchange -> write(exchange, 200, Map.of("status", "ok")));
        server.createContext("/v1/test", exchange -> handle(exchange, token, Main::test));
        server.createContext("/v1/catalog", exchange -> handle(exchange, token, Main::catalog));
        server.createContext("/v1/inspect", exchange -> handle(exchange, token, Main::inspect));
        server.createContext("/v1/preview", exchange -> handle(exchange, token, Main::preview));
        server.createContext("/v1/count", exchange -> handle(exchange, token, Main::count));
        server.createContext("/v1/query", exchange -> handle(exchange, token, Main::query));
        server.createContext("/v1/ddl", exchange -> handle(exchange, token, Main::ddl));
        server.setExecutor(null);
        server.start();
        System.out.println("cdc-jdbc-admin listening on " + host + ":" + port);
    }

    private static Object test(AdminRequest req) throws Exception {
        return Map.of("ok", true, "message", JdbcOps.test(req.dbType, req.connection));
    }

    private static Object catalog(AdminRequest req) throws Exception {
        JdbcOps.CatalogResult result = JdbcOps.catalog(req.dbType, req.connection, req.schemaFilter);
        List<Map<String, Object>> tables = new ArrayList<>();
        for (JdbcOps.TableInfo table : result.tables()) {
            tables.add(Map.of(
                    "schema", table.schema(),
                    "name", table.name(),
                    "primary_key", table.primaryKey()
            ));
        }
        return Map.of("schemas", result.schemas(), "tables", tables);
    }

    private static Object inspect(AdminRequest req) throws Exception {
        require(req.schema, "schema");
        require(req.table, "table");
        JdbcOps.InspectResult result = JdbcOps.inspect(req.dbType, req.connection, req.schema, req.table);
        List<Map<String, Object>> columns = new ArrayList<>();
        for (JdbcOps.ColumnInfo col : result.columns()) {
            Map<String, Object> row = new HashMap<>();
            row.put("name", col.name());
            row.put("data_type", col.dataType());
            row.put("nullable", col.nullable());
            row.put("position", col.position());
            row.put("primary_key", col.primaryKey());
            row.put("pk_position", col.pkPosition());
            row.put("unique", col.unique());
            columns.add(row);
        }
        return Map.of(
                "schema", result.schema(),
                "name", result.name(),
                "primary_key", result.primaryKey(),
                "columns", columns
        );
    }

    private static Object preview(AdminRequest req) throws Exception {
        require(req.schema, "schema");
        require(req.table, "table");
        JdbcOps.PreviewResult result = JdbcOps.preview(
                req.dbType, req.connection, req.schema, req.table, req.page, req.pageSize, req.search
        );
        return Map.of(
                "columns", result.columns(),
                "rows", result.rows(),
                "page", result.page(),
                "page_size", result.pageSize(),
                "has_more", result.hasMore()
        );
    }

    private static Object count(AdminRequest req) throws Exception {
        require(req.schema, "schema");
        if (req.table != null && !req.table.isBlank()) {
            long n = JdbcOps.countTable(req.dbType, req.connection, req.schema, req.table);
            return Map.of("schema", req.schema, "table", req.table, "count", n, "estimated", false);
        }
        return Map.of(
                "schema", req.schema,
                "estimated", false,
                "counts", JdbcOps.counts(req.dbType, req.connection, req.schema)
        );
    }

    private static Object query(AdminRequest req) throws Exception {
        require(req.statement, "statement");
        JdbcOps.QueryResult result = JdbcOps.query(req.dbType, req.connection, req.statement);
        return Map.of("columns", result.columns(), "rows", result.rows());
    }

    private static Object ddl(AdminRequest req) throws Exception {
        require(req.statement, "statement");
        long updateCount = JdbcOps.ddl(req.dbType, req.connection, req.statement, req.schema);
        return Map.of("ok", true, "update_count", updateCount);
    }

    private static void handle(HttpExchange exchange, String token, Handler handler) throws IOException {
        if (!"POST".equalsIgnoreCase(exchange.getRequestMethod())) {
            write(exchange, 405, Map.of("error", "POST required"));
            return;
        }
        if (!authorized(exchange, token)) {
            write(exchange, 401, Map.of("error", "unauthorized"));
            return;
        }
        try {
            AdminRequest req = readRequest(exchange.getRequestBody());
            write(exchange, 200, handler.apply(req));
        } catch (IllegalArgumentException e) {
            write(exchange, 400, Map.of("error", e.getMessage()));
        } catch (Exception e) {
            write(exchange, 502, Map.of("error", e.getMessage() == null ? e.toString() : e.getMessage()));
        }
    }

    private static boolean authorized(HttpExchange exchange, String token) {
        if (token == null || token.isBlank()) {
            return true;
        }
        String header = exchange.getRequestHeaders().getFirst("X-Admin-Token");
        if (token.equals(header)) {
            return true;
        }
        String auth = exchange.getRequestHeaders().getFirst("Authorization");
        return auth != null && auth.equals("Bearer " + token);
    }

    private static AdminRequest readRequest(InputStream in) throws IOException {
        JsonNode root = JSON.readTree(in);
        if (root == null || root.isNull()) {
            throw new IllegalArgumentException("JSON body is required");
        }
        String dbType = text(root, "db_type");
        if (dbType == null || dbType.isBlank()) {
            throw new IllegalArgumentException("db_type is required");
        }
        Map<String, String> connection = new HashMap<>();
        JsonNode conn = root.get("connection");
        if (conn != null && conn.isObject()) {
            Iterator<String> names = conn.fieldNames();
            while (names.hasNext()) {
                String name = names.next();
                JsonNode value = conn.get(name);
                if (value != null && !value.isNull()) {
                    connection.put(name, value.asText());
                }
            }
        }
        List<String> filter = new ArrayList<>();
        JsonNode filterNode = root.get("schema_filter");
        if (filterNode != null && filterNode.isArray()) {
            for (JsonNode item : filterNode) {
                if (item != null && !item.asText().isBlank()) {
                    filter.add(item.asText());
                }
            }
        }
        return new AdminRequest(
                dbType,
                connection,
                text(root, "schema"),
                text(root, "table"),
                text(root, "statement"),
                text(root, "search"),
                filter,
                root.path("page").asLong(0),
                root.path("page_size").asLong(50)
        );
    }

    private static void write(HttpExchange exchange, int status, Object body) throws IOException {
        byte[] bytes = JSON.writeValueAsBytes(body instanceof Map || body instanceof ObjectNode
                ? body
                : JSON.valueToTree(body));
        exchange.getResponseHeaders().set("Content-Type", "application/json");
        exchange.sendResponseHeaders(status, bytes.length);
        try (OutputStream out = exchange.getResponseBody()) {
            out.write(bytes);
        }
    }

    private static String text(JsonNode root, String field) {
        JsonNode node = root.get(field);
        return node == null || node.isNull() ? null : node.asText();
    }

    private static void require(String value, String field) {
        if (value == null || value.isBlank()) {
            throw new IllegalArgumentException(field + " is required");
        }
    }

    private static String env(String key, String fallback) {
        String value = System.getenv(key);
        return value == null || value.isBlank() ? fallback : value.trim();
    }

    private static String firstNonBlank(String... values) {
        if (values == null) {
            return null;
        }
        for (String value : values) {
            if (value != null && !value.isBlank()) {
                return value;
            }
        }
        return null;
    }

    private record AdminRequest(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table,
            String statement,
            String search,
            List<String> schemaFilter,
            long page,
            long pageSize
    ) {}

    @FunctionalInterface
    private interface Handler {
        Object apply(AdminRequest req) throws Exception;
    }
}
