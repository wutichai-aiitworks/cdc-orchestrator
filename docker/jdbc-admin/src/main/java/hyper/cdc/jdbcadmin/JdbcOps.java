package hyper.cdc.jdbcadmin;

import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.DriverManager;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Statement;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Properties;
import java.util.Set;

final class JdbcOps {
    private static final Set<String> SYSTEM_SCHEMAS = Set.of(
            "pg_catalog",
            "information_schema",
            "pg_toast",
            "pg_temp",
            "hyper_cdc",
            "mysql",
            "sys",
            "performance_schema",
            "sysibm",
            "syscat",
            "sysstat"
    );

    private JdbcOps() {}

    static {
        // Shade can drop JDBC ServiceLoader entries; load drivers by name.
        loadDriver("org.postgresql.Driver");
        loadDriver("com.mysql.cj.jdbc.Driver");
        loadDriver("com.microsoft.sqlserver.jdbc.SQLServerDriver");
        loadDriver("oracle.jdbc.OracleDriver");
        loadDriver("org.sqlite.JDBC");
    }

    private static void loadDriver(String className) {
        try {
            Class.forName(className);
        } catch (ClassNotFoundException ignored) {
            // Optional driver not packaged in this build.
        }
    }

    static Connection open(String dbType, Map<String, String> connection) throws SQLException {
        String url = JdbcUrls.jdbcUrl(dbType, connection);
        Properties props = new Properties();
        String user = JdbcUrls.username(connection);
        if (user != null && !user.isBlank()) {
            props.setProperty("user", user);
        }
        props.setProperty("password", JdbcUrls.password(connection));
        if (dbType != null && dbType.trim().equalsIgnoreCase("oracle")) {
            // Slim JREs often lack Oracle TZ region names (ORA-01882).
            props.setProperty("oracle.jdbc.timezoneAsRegion", "false");
        }
        DriverManager.setLoginTimeout(15);
        return DriverManager.getConnection(url, props);
    }

    static String test(String dbType, Map<String, String> connection) throws SQLException {
        try (Connection conn = open(dbType, connection)) {
            boolean ok = conn.isValid(10);
            if (!ok) {
                throw new SQLException("connection established but isValid returned false");
            }
            String host = value(connection, "host", "");
            String port = value(connection, "port", "");
            return "connection successful via jdbc-admin: " + host + ":" + port;
        }
    }

    static CatalogResult catalog(
            String dbType,
            Map<String, String> connection,
            List<String> schemaFilter
    ) throws SQLException {
        Set<String> filter = new LinkedHashSet<>();
        if (schemaFilter != null) {
            for (String s : schemaFilter) {
                if (s != null && !s.isBlank()) {
                    filter.add(s.trim().toLowerCase(Locale.ROOT));
                }
            }
        }
        try (Connection conn = open(dbType, connection)) {
            DatabaseMetaData meta = conn.getMetaData();
            Set<String> schemas = new LinkedHashSet<>();
            List<TableInfo> tables = new ArrayList<>();
            try (ResultSet rs = meta.getTables(null, null, "%", new String[] {"TABLE"})) {
                while (rs.next()) {
                    String schema = schemaName(rs);
                    String name = rs.getString("TABLE_NAME");
                    if (name == null || name.isBlank() || isSystemSchema(schema)) {
                        continue;
                    }
                    schemas.add(schema);
                    if (!filter.isEmpty() && !filter.contains(schema.toLowerCase(Locale.ROOT))) {
                        continue;
                    }
                    tables.add(new TableInfo(schema, name, primaryKeys(meta, schema, name)));
                }
            }
            if (schemas.isEmpty()) {
                try (ResultSet rs = meta.getSchemas()) {
                    while (rs.next()) {
                        String schema = firstNonBlank(rs.getString("TABLE_SCHEM"), rs.getString(1));
                        if (schema != null && !isSystemSchema(schema)) {
                            schemas.add(schema);
                        }
                    }
                }
            }
            return new CatalogResult(new ArrayList<>(schemas), tables);
        }
    }

    static InspectResult inspect(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table
    ) throws SQLException {
        try (Connection conn = open(dbType, connection)) {
            DatabaseMetaData meta = conn.getMetaData();
            List<String> pk = primaryKeys(meta, schema, table);
            List<ColumnInfo> columns = new ArrayList<>();
            int position = 1;
            for (ColumnRow col : loadColumns(meta, schema, table)) {
                boolean isPk = pk.stream().anyMatch(p -> p.equalsIgnoreCase(col.name));
                int pkPos = -1;
                for (int i = 0; i < pk.size(); i++) {
                    if (pk.get(i).equalsIgnoreCase(col.name)) {
                        pkPos = i + 1;
                        break;
                    }
                }
                columns.add(new ColumnInfo(
                        col.name,
                        col.type,
                        col.nullable,
                        position++,
                        isPk,
                        pkPos > 0 ? pkPos : null,
                        isPk
                ));
            }
            return new InspectResult(schema, table, pk, columns);
        }
    }

    static PreviewResult preview(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table,
            long page,
            long pageSize,
            String search
    ) throws SQLException {
        long size = Math.max(1, Math.min(pageSize, 200));
        long offset = Math.max(0, page) * size;
        String qualified = JdbcUrls.qualified(dbType, schema, table);
        String sql = "SELECT * FROM " + qualified;
        try (Connection conn = open(dbType, connection);
             Statement stmt = conn.createStatement()) {
            stmt.setMaxRows((int) Math.min(size + 1 + offset, Integer.MAX_VALUE));
            List<String> columns = new ArrayList<>();
            List<List<Object>> rows = new ArrayList<>();
            try (ResultSet rs = stmt.executeQuery(sql)) {
                ResultSetMetaData md = rs.getMetaData();
                for (int i = 1; i <= md.getColumnCount(); i++) {
                    columns.add(md.getColumnLabel(i));
                }
                long skipped = 0;
                String term = search == null ? "" : search.trim().toLowerCase(Locale.ROOT);
                while (rs.next()) {
                    List<Object> row = new ArrayList<>(columns.size());
                    boolean match = term.isEmpty();
                    for (int i = 1; i <= columns.size(); i++) {
                        Object value = rs.getObject(i);
                        row.add(jsonSafe(value));
                        if (!match && value != null
                                && value.toString().toLowerCase(Locale.ROOT).contains(term)) {
                            match = true;
                        }
                    }
                    if (!match) {
                        continue;
                    }
                    if (skipped < offset) {
                        skipped++;
                        continue;
                    }
                    rows.add(row);
                    if (rows.size() > size) {
                        break;
                    }
                }
            }
            boolean hasMore = rows.size() > size;
            if (hasMore) {
                rows = new ArrayList<>(rows.subList(0, (int) size));
            }
            return new PreviewResult(columns, rows, page, size, hasMore);
        }
    }

    static Map<String, Long> counts(
            String dbType,
            Map<String, String> connection,
            String schema
    ) throws SQLException {
        CatalogResult catalog = catalog(dbType, connection, List.of(schema));
        Map<String, Long> out = new LinkedHashMap<>();
        try (Connection conn = open(dbType, connection);
             Statement stmt = conn.createStatement()) {
            int n = 0;
            for (TableInfo table : catalog.tables) {
                if (n++ >= 80) {
                    break;
                }
                String sql = "SELECT COUNT(*) FROM "
                        + JdbcUrls.qualified(dbType, table.schema, table.name);
                try (ResultSet rs = stmt.executeQuery(sql)) {
                    out.put(table.name, rs.next() ? rs.getLong(1) : 0L);
                } catch (SQLException e) {
                    out.put(table.name, -1L);
                }
            }
        }
        return out;
    }

    static long countTable(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table
    ) throws SQLException {
        String sql = "SELECT COUNT(*) FROM " + JdbcUrls.qualified(dbType, schema, table);
        try (Connection conn = open(dbType, connection);
             Statement stmt = conn.createStatement();
             ResultSet rs = stmt.executeQuery(sql)) {
            return rs.next() ? rs.getLong(1) : 0L;
        }
    }

    static QueryResult query(
            String dbType,
            Map<String, String> connection,
            String statement
    ) throws SQLException {
        String sql = requireSelect(statement);
        try (Connection conn = open(dbType, connection);
             Statement stmt = conn.createStatement()) {
            stmt.setMaxRows(500);
            try (ResultSet rs = stmt.executeQuery(sql)) {
                ResultSetMetaData md = rs.getMetaData();
                List<String> columns = new ArrayList<>();
                for (int i = 1; i <= md.getColumnCount(); i++) {
                    columns.add(md.getColumnLabel(i));
                }
                List<List<Object>> rows = new ArrayList<>();
                while (rs.next()) {
                    List<Object> row = new ArrayList<>(columns.size());
                    for (int i = 1; i <= columns.size(); i++) {
                        row.add(jsonSafe(rs.getObject(i)));
                    }
                    rows.add(row);
                }
                return new QueryResult(columns, rows);
            }
        }
    }

    static long ddl(
            String dbType,
            Map<String, String> connection,
            String statement,
            String schema
    ) throws SQLException {
        if (statement == null || statement.isBlank()) {
            throw new IllegalArgumentException("statement is required");
        }
        try (Connection conn = open(dbType, connection);
             Statement stmt = conn.createStatement()) {
            if (schema != null && !schema.isBlank()) {
                try {
                    stmt.execute("CREATE SCHEMA IF NOT EXISTS " + JdbcUrls.quote(dbType, schema));
                } catch (SQLException ignored) {
                    // Vendor may not support IF NOT EXISTS; CREATE TABLE can still succeed.
                }
            }
            boolean hasResult = stmt.execute(statement);
            if (hasResult) {
                return 0L;
            }
            int count = stmt.getUpdateCount();
            return count < 0 ? 0L : count;
        }
    }

    /** Only SELECT (and WITH…SELECT) may go through /v1/query. DML and DDL stay on /v1/ddl. */
    static String requireSelect(String statement) {
        if (statement == null || statement.isBlank()) {
            throw new IllegalArgumentException("statement is required");
        }
        String sql = statement.trim();
        if (sql.endsWith(";")) {
            sql = sql.substring(0, sql.length() - 1).trim();
        }
        String head = sql.replaceFirst("(?is)^\\s+", "");
        String keyword = head.split("\\s+", 2)[0].toLowerCase(Locale.ROOT);
        if (!keyword.equals("select") && !keyword.equals("with")) {
            throw new IllegalArgumentException("query only accepts SELECT statements");
        }
        return sql;
    }

    private static List<String> primaryKeys(DatabaseMetaData meta, String schema, String table)
            throws SQLException {
        List<String> pk = new ArrayList<>();
        try (ResultSet rs = meta.getPrimaryKeys(null, emptyToNull(schema), table)) {
            Map<Integer, String> ordered = new LinkedHashMap<>();
            while (rs.next()) {
                int seq = rs.getInt("KEY_SEQ");
                ordered.put(seq, rs.getString("COLUMN_NAME"));
            }
            ordered.entrySet().stream()
                    .sorted(Map.Entry.comparingByKey())
                    .forEach(e -> pk.add(e.getValue()));
        }
        if (pk.isEmpty() && schema != null && !schema.isBlank()) {
            try (ResultSet rs = meta.getPrimaryKeys(schema, null, table)) {
                Map<Integer, String> ordered = new LinkedHashMap<>();
                while (rs.next()) {
                    ordered.put(rs.getInt("KEY_SEQ"), rs.getString("COLUMN_NAME"));
                }
                ordered.entrySet().stream()
                        .sorted(Map.Entry.comparingByKey())
                        .forEach(e -> pk.add(e.getValue()));
            }
        }
        return pk;
    }

    private static List<ColumnRow> loadColumns(DatabaseMetaData meta, String schema, String table)
            throws SQLException {
        List<ColumnRow> columns = readColumns(meta.getColumns(null, emptyToNull(schema), table, "%"));
        if (columns.isEmpty() && schema != null && !schema.isBlank()) {
            columns = readColumns(meta.getColumns(schema, null, table, "%"));
        }
        return columns;
    }

    private static List<ColumnRow> readColumns(ResultSet rs) throws SQLException {
        List<ColumnRow> columns = new ArrayList<>();
        try (rs) {
            while (rs.next()) {
                String name = rs.getString("COLUMN_NAME");
                if (name == null) {
                    continue;
                }
                String type = firstNonBlank(rs.getString("TYPE_NAME"), String.valueOf(rs.getInt("DATA_TYPE")));
                boolean nullable = rs.getInt("NULLABLE") != DatabaseMetaData.columnNoNulls;
                columns.add(new ColumnRow(name, type == null ? "" : type, nullable));
            }
        }
        return columns;
    }

    private static String schemaName(ResultSet rs) throws SQLException {
        return firstNonBlank(rs.getString("TABLE_SCHEM"), rs.getString("TABLE_CAT"), "");
    }

    private static boolean isSystemSchema(String schema) {
        if (schema == null || schema.isBlank()) {
            return false;
        }
        String lower = schema.toLowerCase(Locale.ROOT);
        return SYSTEM_SCHEMAS.contains(lower) || lower.startsWith("pg_");
    }

    private static String emptyToNull(String value) {
        return value == null || value.isBlank() ? null : value;
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

    private static String value(Map<String, String> connection, String key, String fallback) {
        if (connection == null || !connection.containsKey(key) || connection.get(key) == null) {
            return fallback;
        }
        String v = connection.get(key).trim();
        return v.isEmpty() ? fallback : v;
    }

    private static Object jsonSafe(Object value) {
        if (value == null) {
            return null;
        }
        if (value instanceof Number || value instanceof Boolean || value instanceof String) {
            return value;
        }
        return String.valueOf(value);
    }

    record CatalogResult(List<String> schemas, List<TableInfo> tables) {}

    record TableInfo(String schema, String name, List<String> primaryKey) {}

    record ColumnInfo(
            String name,
            String dataType,
            boolean nullable,
            int position,
            boolean primaryKey,
            Integer pkPosition,
            boolean unique
    ) {}

    record InspectResult(
            String schema,
            String name,
            List<String> primaryKey,
            List<ColumnInfo> columns
    ) {}

    record PreviewResult(
            List<String> columns,
            List<List<Object>> rows,
            long page,
            long pageSize,
            boolean hasMore
    ) {}

    record QueryResult(List<String> columns, List<List<Object>> rows) {}

    private record ColumnRow(String name, String type, boolean nullable) {}
}
