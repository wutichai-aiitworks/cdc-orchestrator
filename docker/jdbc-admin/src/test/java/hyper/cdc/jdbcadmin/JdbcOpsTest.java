package hyper.cdc.jdbcadmin;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class JdbcOpsTest {
    private static final Map<String, String> CONN = Map.of(
            "jdbc_url", "jdbc:h2:mem:cdc_admin_test;MODE=PostgreSQL;DB_CLOSE_DELAY=-1",
            "username", "sa",
            "password", ""
    );

    @Test
    void catalogInspectPreviewDdlAndCount() throws Exception {
        JdbcOps.ddl("h2", CONN, "CREATE SCHEMA IF NOT EXISTS \"bookings\"", "bookings");
        JdbcOps.ddl(
                "h2",
                CONN,
                "CREATE TABLE IF NOT EXISTS \"bookings\".\"tickets\" (id INT PRIMARY KEY, name VARCHAR(64))",
                "bookings"
        );
        JdbcOps.ddl("h2", CONN, "DELETE FROM \"bookings\".\"tickets\"", null);
        JdbcOps.ddl("h2", CONN, "INSERT INTO \"bookings\".\"tickets\" (id, name) VALUES (1, 'alpha')", null);
        JdbcOps.ddl("h2", CONN, "INSERT INTO \"bookings\".\"tickets\" (id, name) VALUES (2, 'beta')", null);

        String message = JdbcOps.test("h2", CONN);
        assertTrue(message.contains("jdbc-admin"));

        JdbcOps.CatalogResult catalog = JdbcOps.catalog("h2", CONN, List.of("BOOKINGS", "bookings"));
        assertTrue(catalog.tables().stream().anyMatch(t -> t.name().equalsIgnoreCase("TICKETS")));

        JdbcOps.InspectResult inspect = JdbcOps.inspect("h2", CONN, "BOOKINGS", "TICKETS");
        if (inspect.columns().isEmpty()) {
            inspect = JdbcOps.inspect("h2", CONN, "bookings", "tickets");
        }
        assertFalse(inspect.columns().isEmpty());
        assertTrue(inspect.primaryKey().stream().anyMatch(c -> c.equalsIgnoreCase("ID")));

        JdbcOps.PreviewResult preview = JdbcOps.preview("h2", CONN, "bookings", "tickets", 0, 10, "alp");
        if (preview.rows().isEmpty()) {
            preview = JdbcOps.preview("h2", CONN, "BOOKINGS", "TICKETS", 0, 10, "alp");
        }
        assertEquals(1, preview.rows().size());

        long count = JdbcOps.countTable("h2", CONN, "bookings", "tickets");
        if (count == 0) {
            count = JdbcOps.countTable("h2", CONN, "BOOKINGS", "TICKETS");
        }
        assertEquals(2, count);

        JdbcOps.QueryResult queried = JdbcOps.query(
                "h2",
                CONN,
                "SELECT name FROM \"bookings\".\"tickets\" WHERE id = 1"
        );
        if (queried.rows().isEmpty()) {
            queried = JdbcOps.query("h2", CONN, "SELECT name FROM \"BOOKINGS\".\"TICKETS\" WHERE id = 1");
        }
        assertEquals(List.of("NAME"), queried.columns().stream().map(String::toUpperCase).toList());
        assertEquals(1, queried.rows().size());

        long deleted = JdbcOps.ddl("h2", CONN, "DELETE FROM \"bookings\".\"tickets\" WHERE id = 2", null);
        if (deleted == 0) {
            deleted = JdbcOps.ddl("h2", CONN, "DELETE FROM \"BOOKINGS\".\"TICKETS\" WHERE id = 2", null);
        }
        assertEquals(1, deleted);
    }

    @Test
    void oracleDriverIsOnClasspath() throws Exception {
        Class.forName("oracle.jdbc.OracleDriver");
    }

    @Test
    void queryRejectsNonSelect() {
        try {
            JdbcOps.requireSelect("DELETE FROM bookings.tickets");
            throw new AssertionError("expected non-SELECT to be rejected");
        } catch (IllegalArgumentException e) {
            assertTrue(e.getMessage().contains("SELECT"));
        }
    }

    @Test
    void buildsPostgresUrl() {
        String url = JdbcUrls.jdbcUrl("postgres", Map.of(
                "host", "cdc-postgres-source",
                "port", "5432",
                "database", "demo"
        ));
        assertEquals("jdbc:postgresql://cdc-postgres-source:5432/demo", url);
        assertEquals("\"bookings\".\"tickets\"", JdbcUrls.qualified("postgres", "bookings", "tickets"));
    }

    @Test
    void inspectReturnsSizesDefaultsAndIndexes() throws Exception {
        Map<String, String> conn = Map.of(
                "jdbc_url", "jdbc:h2:mem:cdc_admin_ddl;DB_CLOSE_DELAY=-1",
                "username", "sa",
                "password", ""
        );
        JdbcOps.ddl("h2", conn, "CREATE SCHEMA IF NOT EXISTS SHOP", null);
        JdbcOps.ddl("h2", conn, "CREATE TABLE SHOP.PRODUCT ("
                + "ID INT PRIMARY KEY, "
                + "SKU VARCHAR(40) NOT NULL, "
                + "PRICE DECIMAL(10,2) DEFAULT 0, "
                + "BRAND VARCHAR(50), "
                + "CATEGORY VARCHAR(30), "
                + "CONSTRAINT UQ_PRODUCT_SKU UNIQUE (SKU))", null);
        JdbcOps.ddl("h2", conn, "CREATE INDEX IX_PRODUCT_BRAND_CAT ON SHOP.PRODUCT (BRAND, CATEGORY)", null);

        JdbcOps.InspectResult inspect = JdbcOps.inspect("h2", conn, "SHOP", "PRODUCT");

        JdbcOps.ColumnInfo sku = inspect.columns().stream()
                .filter(c -> c.name().equals("SKU")).findFirst().orElseThrow();
        assertEquals(40, sku.size());
        assertFalse(sku.nullable());
        assertTrue(sku.unique(), "single-column unique constraint marks the column unique");

        JdbcOps.ColumnInfo price = inspect.columns().stream()
                .filter(c -> c.name().equals("PRICE")).findFirst().orElseThrow();
        assertEquals(10, price.size());
        assertEquals(2, price.scale());
        assertEquals("0", price.defaultValue());

        JdbcOps.ColumnInfo brand = inspect.columns().stream()
                .filter(c -> c.name().equals("BRAND")).findFirst().orElseThrow();
        assertFalse(brand.unique(), "non-unique index does not mark the column unique");

        JdbcOps.IndexInfo composite = inspect.indexes().stream()
                .filter(i -> i.name().equals("IX_PRODUCT_BRAND_CAT")).findFirst().orElseThrow();
        assertFalse(composite.unique());
        assertEquals(List.of("BRAND", "CATEGORY"), composite.columns());

        assertTrue(inspect.indexes().stream().anyMatch(i ->
                i.unique() && i.columns().equals(List.of("SKU"))));
    }

    @Test
    void openStreamReadsEveryRowOnceAndHonoursMaxRows() throws Exception {
        Map<String, String> conn = Map.of(
                "jdbc_url", "jdbc:h2:mem:cdc_admin_stream;MODE=PostgreSQL;DB_CLOSE_DELAY=-1",
                "username", "sa",
                "password", ""
        );
        JdbcOps.ddl("h2", conn, "CREATE SCHEMA IF NOT EXISTS \"s\"", "s");
        JdbcOps.ddl(
                "h2",
                conn,
                "CREATE TABLE IF NOT EXISTS \"s\".\"big\" (id INT PRIMARY KEY, name VARCHAR(16))",
                "s"
        );
        JdbcOps.ddl("h2", conn, "DELETE FROM \"s\".\"big\"", null);
        JdbcOps.ddl(
                "h2",
                conn,
                "INSERT INTO \"s\".\"big\" SELECT X, 'n' || X FROM SYSTEM_RANGE(1, 2500)",
                null
        );

        // fetch_size smaller than the table so the cursor makes several round trips.
        try (JdbcOps.RowStream rows = JdbcOps.openStream("h2", conn, "s", "big", 0, 300)) {
            assertEquals(2, rows.columns().size());
            java.util.Set<Object> ids = new java.util.HashSet<>();
            List<Object> row;
            while ((row = rows.next()) != null) {
                assertTrue(ids.add(row.get(0)), "row read twice: " + row);
            }
            assertEquals(2500, ids.size());
        }

        try (JdbcOps.RowStream rows = JdbcOps.openStream("h2", conn, "s", "big", 7, 0)) {
            int n = 0;
            while (rows.next() != null) {
                n++;
            }
            assertEquals(7, n);
        }
    }
}
