# Debezium Connect plus Confluent Avro converter and JDBC source (trigger CDC).
# Build context is the repo root so this image can compile kafka-connect-jdbc/.
FROM maven:3.9.9-eclipse-temurin-17 AS deps
WORKDIR /build
COPY docker/connect/maven-settings.xml /root/.m2/settings.xml
COPY docker/connect/pom.xml .
COPY docker/connect/jdbc-pom.xml .
# Download all jdbc deps EXCEPT the connector jar itself (replaced by local fork below).
RUN mvn -B -q -f pom.xml -DincludeScope=runtime dependency:copy-dependencies -DoutputDirectory=/jars/avro
RUN mvn -B -q -f jdbc-pom.xml -DincludeScope=runtime dependency:copy-dependencies \
      -DoutputDirectory=/jars/jdbc \
      -DexcludeArtifactIds=kafka-connect-jdbc \
 && rm -f /jars/jdbc/kafka-connect-jdbc-*.jar

# Compile the vendored Hyper fork (OracleDatabaseDialect: timezoneAsRegion fix).
# compile + jar:jar only — skip Hub/assembly packaging so the image gets the same
# thin connector jar previously copied as kafka-connect-jdbc-local.jar.
COPY kafka-connect-jdbc kafka-connect-jdbc
RUN mvn -B -f kafka-connect-jdbc/pom.xml \
      -DskipTests \
      -Dmaven.test.skip=true \
      -Dcheckstyle.skip=true \
      compile jar:jar \
 && cp kafka-connect-jdbc/target/kafka-connect-jdbc-10.9.7.jar /jars/jdbc/kafka-connect-jdbc.jar

FROM quay.io/debezium/connect:2.7
USER root
COPY --from=deps /jars/avro /kafka/connect/avro-converter
COPY --from=deps /jars/jdbc /kafka/connect/kafka-connect-jdbc
# Optional: unpack Apache Iceberg Kafka Connect runtime into /kafka/connect/iceberg-sink
# so source_to_lake Iceberg sinks can start. See design compendium Ch.12 (#datalake).
USER kafka
