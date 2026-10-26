package hyper.cdc.jdbcadmin;

import java.util.Locale;
import java.util.Map;

final class JdbcUrls {
    private JdbcUrls() {}

    static String jdbcUrl(String dbType, Map<String, String> connection) {
        if (connection == null) {
            throw new IllegalArgumentException("connection is required");
        }
        String explicit = first(connection, "jdbc_url", "jdbcUrl");
        if (explicit != null && !explicit.isBlank()) {
            return explicit.trim();
        }
        String type = dbType == null ? "" : dbType.trim().toLowerCase(Locale.ROOT);
        String host = or(connection, "host", "localhost");
        String port = or(connection, "port", "");
        String database = or(connection, "database", "");
        return switch (type) {
            case "postgres", "postgresql" -> {
                String p = port.isEmpty() ? "5432" : port;
                yield "jdbc:postgresql://" + host + ":" + p + "/" + database;
            }
            case "mysql", "mariadb" -> {
                String p = port.isEmpty() ? "3306" : port;
                yield "jdbc:mysql://" + host + ":" + p + "/" + database
                        + "?useSSL=false&allowPublicKeyRetrieval=true";
            }
            case "sqlserver", "mssql" -> {
                String p = port.isEmpty() ? "1433" : port;
                yield "jdbc:sqlserver://" + host + ":" + p
                        + ";databaseName=" + database
                        + ";encrypt=false;trustServerCertificate=true";
            }
            case "sqlite" -> "jdbc:sqlite:" + (database.isEmpty() ? ":memory:" : database);
            case "oracle" -> {
                String p = port.isEmpty() ? "1521" : port;
                yield "jdbc:oracle:thin:@//" + host + ":" + p + "/" + database;
            }
            case "as400" -> {
                String p = port.isEmpty() ? "446" : port;
                yield "jdbc:as400://" + host + ":" + p + "/" + database;
            }
            case "db2" -> {
                String p = port.isEmpty() ? "50000" : port;
                yield "jdbc:db2://" + host + ":" + p + "/" + database;
            }
            case "informix" -> {
                String p = port.isEmpty() ? "9088" : port;
                String server = or(connection, "server", "informixserver");
                yield "jdbc:informix-sqli://" + host + ":" + p + "/" + database
                        + ":informixserver=" + server;
            }
            case "h2" -> {
                if (database.isEmpty()) {
                    yield "jdbc:h2:mem:cdc_admin;MODE=PostgreSQL;DB_CLOSE_DELAY=-1";
                }
                yield database.startsWith("jdbc:") ? database : "jdbc:h2:" + database;
            }
            default -> throw new IllegalArgumentException(
                    "unsupported database type for JDBC Type 4: " + dbType);
        };
    }

    static String quote(String dbType, String ident) {
        if (ident == null || ident.isBlank()) {
            throw new IllegalArgumentException("identifier is required");
        }
        String type = dbType == null ? "" : dbType.trim().toLowerCase(Locale.ROOT);
        return switch (type) {
            case "mysql", "mariadb" -> "`" + ident.replace("`", "``") + "`";
            case "sqlserver", "mssql" -> "[" + ident.replace("]", "]]") + "]";
            default -> "\"" + ident.replace("\"", "\"\"") + "\"";
        };
    }

    static String qualified(String dbType, String schema, String table) {
        if (schema == null || schema.isBlank()) {
            return quote(dbType, table);
        }
        return quote(dbType, schema) + "." + quote(dbType, table);
    }

    static String username(Map<String, String> connection) {
        return first(connection, "username", "user");
    }

    static String password(Map<String, String> connection) {
        String p = first(connection, "password", "pass");
        return p == null ? "" : p;
    }

    private static String or(Map<String, String> connection, String key, String fallback) {
        String v = first(connection, key);
        return v == null || v.isBlank() ? fallback : v.trim();
    }

    private static String first(Map<String, String> connection, String... keys) {
        if (connection == null) {
            return null;
        }
        for (String key : keys) {
            if (connection.containsKey(key) && connection.get(key) != null) {
                return connection.get(key);
            }
        }
        return null;
    }
}
