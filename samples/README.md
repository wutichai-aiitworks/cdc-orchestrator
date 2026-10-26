# Demo dumps (Postgres Professional bookings)

Gzipped `pg_dump` files for the `demo` database (`bookings` schema).

| File | Window | Typical use |
|------|--------|-------------|
| `demo-3m.sql.gz` | 3 months | Default Compose init |
| `demo-6m.sql.gz` | 6 months | Larger CDC soak |
| `demo-1y.sql.gz` | 1 year | Heavy source load |
| `demo-2y.sql.gz` | 2 years | Largest source load |

Set `CDC_DEMO_DUMP_SIZE` to `3m`, `6m`, `1y`, or `2y` before the **first** source Postgres start (init runs only on an empty volume).

These dumps load into the **source** only. The target `demo` database stays empty for CDC to fill.

These dumps are not committed; keep them in this folder locally.
