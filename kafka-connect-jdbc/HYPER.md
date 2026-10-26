# Hyper fork of kafka-connect-jdbc

This directory is a **modified** copy of Confluent's
[kafka-connect-jdbc](https://github.com/confluentinc/kafka-connect-jdbc)
**v10.9.7**, vendored into `hyper-cdc-orchestrator` so the Connect image can
rebuild the connector from source.

Upstream license: [Confluent Community License](LICENSE). Copyright Confluent Inc.

## Hyper modifications

1. `OracleDatabaseDialect.addConnectionProperties()` sets
   `oracle.jdbc.timezoneAsRegion=false` so the Oracle thin driver does not throw
   `ORA-01882` ("timezone region not found") when the JVM default timezone (for
   example `UTC` inside Docker) is missing from Oracle's region table.
2. `pom.xml` lists Maven Central **before** Confluent so image builds do not
   404-probe `packages.confluent.io` for every Central artifact.

## Build

The `cdc-connect` image packages this tree (`docker/connect/Dockerfile`).
Do not copy a prebuilt jar from a sibling checkout.
