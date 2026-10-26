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
}
