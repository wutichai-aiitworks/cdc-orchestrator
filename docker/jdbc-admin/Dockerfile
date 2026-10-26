FROM maven:3.9.9-eclipse-temurin-17 AS build
WORKDIR /build
COPY pom.xml .
COPY src ./src
RUN mvn -q -DskipTests package

FROM eclipse-temurin:17-jre
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/cdc-jdbc-admin-1.0.0.jar /app/jdbc-admin.jar
EXPOSE 8084
ENV JDBC_ADMIN_HOST=0.0.0.0 \
    JDBC_ADMIN_PORT=8084 \
    TZ=UTC \
    JAVA_TOOL_OPTIONS="-Duser.timezone=UTC -Doracle.jdbc.timezoneAsRegion=false"
ENTRYPOINT ["java", "-Xms64m", "-Xmx256m", "-jar", "/app/jdbc-admin.jar"]
