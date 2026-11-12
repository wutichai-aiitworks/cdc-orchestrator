package hyper.cdc.jdbcadmin;

import java.io.InputStream;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.sql.Blob;
import java.sql.Clob;
import java.sql.Connection;
import java.sql.PreparedStatement;
import java.sql.Types;
import java.time.LocalDate;
import java.time.LocalDateTime;
import java.time.LocalTime;
import java.time.OffsetDateTime;
import java.time.format.DateTimeParseException;
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
            try (ResultSet rs = meta.getSchemas()) {
                while (rs.next()) {
                    String schema = firstNonBlank(rs.getString("TABLE_SCHEM"), rs.getString(1));
                    if (schema == null || schema.isBlank() || isSystemSchema(schema)) {
                        continue;
                    }
                    if (!filter.isEmpty()
                            && !filter.contains(schema.toLowerCase(Locale.ROOT))) {
                        continue;
                    }
                    schemas.add(schema);
                }
            }
            try (ResultSet rs = meta.getTables(null, null, "%", new String[] {"TABLE"})) {
                while (rs.next()) {
                    String schema = schemaName(rs);
                    String name = rs.getString("TABLE_NAME");
                    if (name == null || name.isBlank() || isSystemSchema(schema)) {
                        continue;
                    }
                    if (!filter.isEmpty() && !filter.contains(schema.toLowerCase(Locale.ROOT))) {
                        continue;
                    }
                    schemas.add(schema);
                    tables.add(new TableInfo(schema, name, primaryKeys(meta, schema, name)));
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
            List<IndexInfo> indexes = loadIndexes(meta, schema, table);
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
                boolean unique = (isPk && pk.size() == 1) || indexes.stream().anyMatch(ix ->
                        ix.unique() && ix.filter() == null && ix.columns().size() == 1
                                && ix.columns().get(0).equalsIgnoreCase(col.name));
                columns.add(new ColumnInfo(
                        col.name,
                        col.type,
                        col.nullable,
                        position++,
                        isPk,
                        pkPos > 0 ? pkPos : null,
                        unique,
                        col.size,
                        col.scale,
                        col.defaultValue,
                        col.autoIncrement
                ));
            }
            return new InspectResult(schema, table, pk, columns, indexes);
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

    /** Default rows per round trip for {@link #openStream}. */
    static final int STREAM_FETCH_SIZE = 1000;

    /**
     * Open a single forward-only {@code SELECT *} over a table for streaming copies.
     *
     * <p>Unlike {@link #preview}, which re-runs the query and skips {@code page * size} rows
     * for every page (quadratic on large tables), this issues one statement and reads it
     * with a cursor, so the source sees exactly one read of each row. {@code maxRows <= 0}
     * means no cap. The caller must close the returned stream.
     */
    static RowStream openStream(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table,
            long maxRows,
            int fetchSize
    ) throws SQLException {
        return openStream(dbType, connection, schema, table, maxRows, fetchSize, List.of(), false);
    }

    /**
     * Like {@link #openStream(String, Map, String, String, long, int)}, optionally reading only
     * {@code columns} (quoted, in that order) and emitting {@linkplain #canonical canonical}
     * values. Record diff uses both: keys + compared columns only, comparable across engines.
     */
    static RowStream openStream(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table,
            long maxRows,
            int fetchSize,
            List<String> columns,
            boolean canonical
    ) throws SQLException {
        String sql = "SELECT " + projection(dbType, columns) + " FROM "
                + JdbcUrls.qualified(dbType, schema, table);
        Connection conn = open(dbType, connection);
        try {
            // PostgreSQL only streams with a cursor when autocommit is off; harmless elsewhere.
            conn.setAutoCommit(false);
            Statement stmt = conn.createStatement(ResultSet.TYPE_FORWARD_ONLY, ResultSet.CONCUR_READ_ONLY);
            stmt.setFetchSize(fetchSize > 0 ? fetchSize : STREAM_FETCH_SIZE);
            if (maxRows > 0) {
                stmt.setMaxRows((int) Math.min(maxRows, Integer.MAX_VALUE));
            }
            ResultSet rs = stmt.executeQuery(sql);
            ResultSetMetaData md = rs.getMetaData();
            List<String> labels = new ArrayList<>();
            for (int i = 1; i <= md.getColumnCount(); i++) {
                labels.add(md.getColumnLabel(i));
            }
            return new RowStream(conn, stmt, rs, labels, canonical ? sqlTypes(md) : null);
        } catch (SQLException | RuntimeException e) {
            try {
                conn.close();
            } catch (SQLException ignored) {
                // already failing
            }
            throw e;
        }
    }

    /** Forward-only row cursor returned by {@link #openStream}. */
    static final class RowStream implements AutoCloseable {
        private final Connection conn;
        private final Statement stmt;
        private final ResultSet rs;
        private final List<String> columns;
        /** JDBC types per column when emitting canonical values; {@code null} = plain JSON-safe. */
        private final int[] canonicalTypes;

        private RowStream(Connection conn, Statement stmt, ResultSet rs, List<String> columns, int[] canonicalTypes) {
            this.conn = conn;
            this.stmt = stmt;
            this.rs = rs;
            this.columns = columns;
            this.canonicalTypes = canonicalTypes;
        }

        List<String> columns() {
            return columns;
        }

        /** Next row as JSON-safe values, or {@code null} when the result set is exhausted. */
        List<Object> next() throws SQLException {
            if (!rs.next()) {
                return null;
            }
            List<Object> row = new ArrayList<>(columns.size());
            for (int i = 1; i <= columns.size(); i++) {
                row.add(canonicalTypes == null
                        ? jsonSafe(rs.getObject(i))
                        : canonical(rs, i, canonicalTypes[i - 1]));
            }
            return row;
        }

        @Override
        public void close() {
            try {
                rs.close();
            } catch (SQLException ignored) {
                // closing
            }
            try {
                stmt.close();
            } catch (SQLException ignored) {
                // closing
            }
            try {
                // Read-only work: never commit anything on the source.
                conn.rollback();
            } catch (SQLException ignored) {
                // closing
            }
            try {
                conn.close();
            } catch (SQLException ignored) {
                // closing
            }
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
                // Read in ascending column order: Oracle returns COLUMN_DEF as a LONG,
                // which must be consumed before any later column.
                String name = rs.getString("COLUMN_NAME");
                int dataType = rs.getInt("DATA_TYPE");
                String typeName = rs.getString("TYPE_NAME");
                Integer size = intOrNull(rs, "COLUMN_SIZE");
                Integer scale = intOrNull(rs, "DECIMAL_DIGITS");
                boolean nullable = rs.getInt("NULLABLE") != DatabaseMetaData.columnNoNulls;
                String defaultValue = trimToNull(rs.getString("COLUMN_DEF"));
                boolean autoIncrement = false;
                try {
                    autoIncrement = "YES".equalsIgnoreCase(rs.getString("IS_AUTOINCREMENT"));
                } catch (SQLException ignored) {
                    // Older drivers do not expose IS_AUTOINCREMENT.
                }
                if (name == null) {
                    continue;
                }
                String type = firstNonBlank(typeName, String.valueOf(dataType));
                columns.add(new ColumnRow(
                        name, type == null ? "" : type, nullable, size, scale, defaultValue, autoIncrement));
            }
        }
        return columns;
    }

    /**
     * Index and unique-constraint metadata. {@code approximate=true} matters on Oracle:
     * with {@code false} the driver may run ANALYZE TABLE against the source.
     */
    private static List<IndexInfo> loadIndexes(DatabaseMetaData meta, String schema, String table) {
        List<IndexInfo> out = readIndexes(meta, null, emptyToNull(schema), table);
        if (out.isEmpty() && schema != null && !schema.isBlank()) {
            out = readIndexes(meta, schema, null, table);
        }
        return out;
    }

    private static List<IndexInfo> readIndexes(
            DatabaseMetaData meta, String catalog, String schema, String table) {
        Map<String, IndexBuilder> byName = new LinkedHashMap<>();
        try (ResultSet rs = meta.getIndexInfo(catalog, schema, table, false, true)) {
            while (rs.next()) {
                boolean nonUnique = rs.getBoolean("NON_UNIQUE");
                String name = rs.getString("INDEX_NAME");
                short type = rs.getShort("TYPE");
                int ordinal = rs.getInt("ORDINAL_POSITION");
                String column = rs.getString("COLUMN_NAME");
                String order = rs.getString("ASC_OR_DESC");
                String filter = null;
                try {
                    filter = trimToNull(rs.getString("FILTER_CONDITION"));
                } catch (SQLException ignored) {
                    // Not all drivers expose FILTER_CONDITION.
                }
                if (name == null || type == DatabaseMetaData.tableIndexStatistic || column == null) {
                    continue;
                }
                IndexBuilder b = byName.computeIfAbsent(name, n -> new IndexBuilder(n, !nonUnique));
                b.columns.put(ordinal, column);
                if ("D".equalsIgnoreCase(order)) {
                    b.descending.add(column);
                }
                if (filter != null) {
                    b.filter = filter;
                }
            }
        } catch (SQLException e) {
            // Index metadata is best effort; the table structure is still useful without it.
            return List.of();
        }
        List<IndexInfo> out = new ArrayList<>();
        for (IndexBuilder b : byName.values()) {
            List<String> cols = new ArrayList<>();
            b.columns.entrySet().stream()
                    .sorted(Map.Entry.comparingByKey())
                    .forEach(e -> cols.add(e.getValue()));
            out.add(new IndexInfo(b.name, b.unique, cols, new ArrayList<>(b.descending), b.filter));
        }
        return out;
    }

    private static Integer intOrNull(ResultSet rs, String column) throws SQLException {
        int v = rs.getInt(column);
        return rs.wasNull() ? null : v;
    }

    private static String trimToNull(String value) {
        if (value == null) {
            return null;
        }
        String t = value.trim();
        return t.isEmpty() ? null : t;
    }

    private static final class IndexBuilder {
        final String name;
        final boolean unique;
        final Map<Integer, String> columns = new LinkedHashMap<>();
        final Set<String> descending = new LinkedHashSet<>();
        String filter;

        IndexBuilder(String name, boolean unique) {
            this.name = name;
            this.unique = unique;
        }
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

    /** {@code *} or the quoted column list, in the given order. */
    private static String projection(String dbType, List<String> columns) {
        if (columns == null || columns.isEmpty()) {
            return "*";
        }
        List<String> quoted = new ArrayList<>(columns.size());
        for (String c : columns) {
            quoted.add(JdbcUrls.quote(dbType, c));
        }
        return String.join(", ", quoted);
    }

    private static int[] sqlTypes(ResultSetMetaData md) throws SQLException {
        int[] types = new int[md.getColumnCount()];
        for (int i = 0; i < types.length; i++) {
            types[i] = md.getColumnType(i + 1);
        }
        return types;
    }

    /**
     * One column as a string that compares equal across database engines, or {@code null}.
     *
     * <p>Numbers: plain decimal without trailing zeros ({@code 10.50} → {@code 10.5}, {@code 1E+3}
     * → {@code 1000}). Timestamps: ISO local date-time ({@code 2024-01-02T03:04:05.5}); zoned
     * timestamps: UTC instant. Dates / times: ISO. Booleans: {@code true}/{@code false}. CLOB:
     * text. BLOB / binary: {@code sha256:<hex>} so large values never travel. Everything else:
     * its string form.
     */
    static String canonical(ResultSet rs, int i, int sqlType) throws SQLException {
        switch (sqlType) {
            case Types.TIMESTAMP: {
                java.sql.Timestamp t = rs.getTimestamp(i);
                return t == null ? null : t.toLocalDateTime().toString();
            }
            case Types.TIMESTAMP_WITH_TIMEZONE: {
                try {
                    OffsetDateTime t = rs.getObject(i, OffsetDateTime.class);
                    return t == null ? null : t.toInstant().toString();
                } catch (SQLException | RuntimeException e) {
                    return rs.getString(i);
                }
            }
            case Types.DATE: {
                java.sql.Date d = rs.getDate(i);
                return d == null ? null : d.toLocalDate().toString();
            }
            case Types.TIME: {
                java.sql.Time t = rs.getTime(i);
                return t == null ? null : t.toLocalTime().toString();
            }
            case Types.BINARY:
            case Types.VARBINARY:
            case Types.LONGVARBINARY: {
                byte[] b = rs.getBytes(i);
                return b == null ? null : "sha256:" + hex(digest().digest(b));
            }
            default:
                return canonicalValue(rs.getObject(i));
        }
    }

    static String canonicalValue(Object v) throws SQLException {
        if (v == null) {
            return null;
        }
        if (v instanceof String s) {
            return s;
        }
        if (v instanceof BigDecimal d) {
            return plain(d);
        }
        if (v instanceof Double || v instanceof Float) {
            double x = ((Number) v).doubleValue();
            return Double.isNaN(x) || Double.isInfinite(x) ? String.valueOf(x) : plain(BigDecimal.valueOf(x));
        }
        if (v instanceof Long || v instanceof Integer || v instanceof Short || v instanceof Byte
                || v instanceof BigInteger) {
            return v.toString();
        }
        if (v instanceof Number n) {
            return plain(new BigDecimal(n.toString()));
        }
        if (v instanceof Boolean b) {
            return b.toString();
        }
        if (v instanceof java.sql.Timestamp t) {
            return t.toLocalDateTime().toString();
        }
        if (v instanceof java.sql.Date d) {
            return d.toLocalDate().toString();
        }
        if (v instanceof java.sql.Time t) {
            return t.toLocalTime().toString();
        }
        if (v instanceof OffsetDateTime t) {
            return t.toInstant().toString();
        }
        if (v instanceof byte[] b) {
            return "sha256:" + hex(digest().digest(b));
        }
        if (v instanceof Clob c) {
            long len = c.length();
            return len == 0 ? "" : c.getSubString(1, (int) Math.min(len, Integer.MAX_VALUE));
        }
        if (v instanceof Blob blob) {
            MessageDigest md = digest();
            try (InputStream in = blob.getBinaryStream()) {
                byte[] buf = new byte[1 << 16];
                int n;
                while ((n = in.read(buf)) > 0) {
                    md.update(buf, 0, n);
                }
            } catch (java.io.IOException e) {
                throw new SQLException("BLOB read failed: " + e.getMessage(), e);
            }
            return "sha256:" + hex(md.digest());
        }
        return String.valueOf(v);
    }

    private static String plain(BigDecimal d) {
        return d.signum() == 0 ? "0" : d.stripTrailingZeros().toPlainString();
    }

    private static MessageDigest digest() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String hex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16)).append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    /** Max key tuples per {@link #lookup} statement (bind-parameter limits: Oracle/MSSQL ~2000). */
    static final int LOOKUP_BATCH = 200;

    /**
     * Re-read rows by key: {@code SELECT columns FROM t WHERE (k1 = ? AND k2 = ?) OR ...}.
     * Keys arrive as {@linkplain #canonical canonical} strings and are bound with the key
     * column's own JDBC type, so the key index is used. A key that cannot be converted to
     * that type (e.g. {@code "abc"} for an INT key) cannot exist and is skipped. Returns the
     * found rows as canonical values, {@code columns} order; at most one statement per
     * {@link #LOOKUP_BATCH} keys, all on one read-only connection.
     */
    static LookupResult lookup(
            String dbType,
            Map<String, String> connection,
            String schema,
            String table,
            List<String> keyColumns,
            List<String> columns,
            List<List<String>> keys
    ) throws SQLException {
        if (keyColumns == null || keyColumns.isEmpty()) {
            throw new IllegalArgumentException("key_columns is required");
        }
        List<String> select = columns == null || columns.isEmpty() ? keyColumns : columns;
        String qualified = JdbcUrls.qualified(dbType, schema, table);
        List<List<String>> rows = new ArrayList<>();
        try (Connection conn = open(dbType, connection)) {
            conn.setReadOnly(true);
            int[] keyTypes;
            try (Statement probe = conn.createStatement();
                 ResultSet rs = probe.executeQuery(
                         "SELECT " + projection(dbType, keyColumns) + " FROM " + qualified + " WHERE 1 = 0")) {
                keyTypes = sqlTypes(rs.getMetaData());
            }
            String where = "(" + String.join(" AND ", keyColumns.stream()
                    .map(c -> JdbcUrls.quote(dbType, c) + " = ?").toList()) + ")";
            String head = "SELECT " + projection(dbType, select) + " FROM " + qualified + " WHERE ";
            List<Object[]> bound = new ArrayList<>();
            for (List<String> key : keys == null ? List.<List<String>>of() : keys) {
                Object[] values = bindable(key, keyTypes);
                if (values != null) {
                    bound.add(values);
                }
            }
            for (int from = 0; from < bound.size(); from += LOOKUP_BATCH) {
                List<Object[]> batch = bound.subList(from, Math.min(bound.size(), from + LOOKUP_BATCH));
                String sql = head + String.join(" OR ", java.util.Collections.nCopies(batch.size(), where));
                try (PreparedStatement ps = conn.prepareStatement(sql)) {
                    int p = 1;
                    for (Object[] values : batch) {
                        for (Object v : values) {
                            ps.setObject(p++, v);
                        }
                    }
                    try (ResultSet rs = ps.executeQuery()) {
                        int[] types = sqlTypes(rs.getMetaData());
                        while (rs.next()) {
                            List<String> row = new ArrayList<>(types.length);
                            for (int i = 1; i <= types.length; i++) {
                                row.add(canonical(rs, i, types[i - 1]));
                            }
                            rows.add(row);
                        }
                    }
                }
            }
            try {
                conn.rollback();
            } catch (SQLException ignored) {
                // autocommit connection: nothing to roll back
            }
        }
        return new LookupResult(select, rows);
    }

    /** Canonical key strings → values of each key column's JDBC type; {@code null} = unconvertible. */
    static Object[] bindable(List<String> key, int[] types) {
        if (key == null || key.size() != types.length) {
            return null;
        }
        Object[] out = new Object[types.length];
        for (int i = 0; i < types.length; i++) {
            String s = key.get(i);
            if (s == null) {
                return null;
            }
            try {
                out[i] = switch (types[i]) {
                    case Types.TINYINT, Types.SMALLINT, Types.INTEGER, Types.BIGINT ->
                            new BigDecimal(s).toBigIntegerExact().longValueExact();
                    case Types.NUMERIC, Types.DECIMAL, Types.REAL, Types.FLOAT, Types.DOUBLE ->
                            new BigDecimal(s);
                    case Types.BIT, Types.BOOLEAN -> s.equals("1") || Boolean.parseBoolean(s);
                    case Types.DATE -> java.sql.Date.valueOf(LocalDate.parse(s.length() > 10 ? s.substring(0, 10) : s));
                    case Types.TIMESTAMP -> java.sql.Timestamp.valueOf(
                            s.length() == 10 ? LocalDate.parse(s).atStartOfDay() : LocalDateTime.parse(s));
                    case Types.TIME -> java.sql.Time.valueOf(LocalTime.parse(s));
                    default -> s;
                };
            } catch (ArithmeticException | NumberFormatException | DateTimeParseException e) {
                return null;
            }
        }
        return out;
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
            boolean unique,
            Integer size,
            Integer scale,
            String defaultValue,
            boolean autoIncrement
    ) {}

    record IndexInfo(
            String name,
            boolean unique,
            List<String> columns,
            List<String> descending,
            String filter
    ) {}

    record InspectResult(
            String schema,
            String name,
            List<String> primaryKey,
            List<ColumnInfo> columns,
            List<IndexInfo> indexes
    ) {}

    record PreviewResult(
            List<String> columns,
            List<List<Object>> rows,
            long page,
            long pageSize,
            boolean hasMore
    ) {}

    record QueryResult(List<String> columns, List<List<Object>> rows) {}

    record LookupResult(List<String> columns, List<List<String>> rows) {}

    private record ColumnRow(
            String name,
            String type,
            boolean nullable,
            Integer size,
            Integer scale,
            String defaultValue,
            boolean autoIncrement
    ) {}
}
