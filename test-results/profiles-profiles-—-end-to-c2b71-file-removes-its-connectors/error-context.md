# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: profiles.spec.ts >> profiles — end-to-end replication (ORCH_TEST_E2E=true) >> KNOWN BUG: deleting the profile removes its connectors
- Location: tests/api/profiles.spec.ts:369:7

# Error details

```
Error: expect(received).not.toContain(expected) // indexOf

Expected substring: not "\"cdc-pw-mvxizvb1-e2e\""
Received string:        "{\"connectors\":{\"cdc-demo\":{\"info\":{\"config\":{\"connector.class\":\"io.debezium.connector.postgresql.PostgresConnector\",\"database.dbname\":\"demo\",\"database.hostname\":\"cdc-postgres-source\",\"database.password\":\"demo\",\"database.port\":\"5432\",\"database.user\":\"demo\",\"key.converter\":\"org.apache.kafka.connect.json.JsonConverter\",\"key.converter.schemas.enable\":\"true\",\"name\":\"cdc-demo\",\"plugin.name\":\"pgoutput\",\"publication.autocreate.mode\":\"disabled\",\"publication.name\":\"dbz_publication\",\"schema.include.list\":\"bookings\",\"slot.name\":\"cdc_demo\",\"snapshot.mode\":\"initial\",\"tasks.max\":\"1\",\"topic.prefix\":\"demo\",\"value.converter\":\"org.apache.kafka.connect.json.JsonConverter\",\"value.converter.schemas.enable\":\"true\"},\"name\":\"cdc-demo\",\"tasks\":[{\"connector\":\"cdc-demo\",\"task\":0}],\"type\":\"source\"},\"status\":{\"connector\":{\"state\":\"RUNNING\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-demo\",\"tasks\":[{\"id\":0,\"state\":\"FAILED\",\"trace\":\"io.debezium.DebeziumException: Couldn't obtain encoding for database demo\\n\\tat io.debezium.connector.postgresql.connection.PostgresConnection.getDatabaseCharset(PostgresConnection.java:599)\\n\\tat io.debezium.connector.postgresql.PostgresConnectorTask.start(PostgresConnectorTask.java:91)\\n\\tat io.debezium.connector.common.BaseSourceTask.start(BaseSourceTask.java:248)\\n\\tat org.apache.kafka.connect.runtime.AbstractWorkerSourceTask.initializeAndStart(AbstractWorkerSourceTask.java:280)\\n\\tat org.apache.kafka.connect.runtime.WorkerTask.doRun(WorkerTask.java:202)\\n\\tat org.apache.kafka.connect.runtime.WorkerTask.run(WorkerTask.java:259)\\n\\tat org.apache.kafka.connect.runtime.AbstractWorkerSourceTask.run(AbstractWorkerSourceTask.java:77)\\n\\tat org.apache.kafka.connect.runtime.isolation.Plugins.lambda$withClassLoader$1(Plugins.java:237)\\n\\tat java.base/java.util.concurrent.Executors$RunnableAdapter.call(Executors.java:572)\\n\\tat java.base/java.util.concurrent.FutureTask.run(FutureTask.java:317)\\n\\tat java.base/java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1144)\\n\\tat java.base/java.util.concurrent.ThreadPoolExecutor$Worker.run(ThreadPoolExecutor.java:642)\\n\\tat java.base/java.lang.Thread.run(Thread.java:1583)\\nCaused by: org.postgresql.util.PSQLException: The connection attempt failed.\\n\\tat org.postgresql.core.v3.ConnectionFactoryImpl.openConnectionImpl(ConnectionFactoryImpl.java:354)\\n\\tat org.postgresql.core.ConnectionFactory.openConnection(ConnectionFactory.java:54)\\n\\tat org.postgresql.jdbc.PgConnection.<init>(PgConnection.java:263)\\n\\tat org.postgresql.Driver.makeConnection(Driver.java:443)\\n\\tat org.postgresql.Driver.connect(Driver.java:297)\\n\\tat io.debezium.jdbc.JdbcConnection.lambda$patternBasedFactory$1(JdbcConnection.java:246)\\n\\tat io.debezium.jdbc.JdbcConnection$ConnectionFactoryDecorator.connect(JdbcConnection.java:132)\\n\\tat io.debezium.jdbc.JdbcConnection.establishConnection(JdbcConnection.java:920)\\n\\tat io.debezium.jdbc.JdbcConnection.connection(JdbcConnection.java:904)\\n\\tat io.debezium.jdbc.JdbcConnection.connection(JdbcConnection.java:898)\\n\\tat io.debezium.connector.postgresql.connection.PostgresConnection.getDatabaseCharset(PostgresConnection.java:596)\\n\\t... 12 more\\nCaused by: java.net.UnknownHostException: cdc-postgres-source\\n\\tat java.base/sun.nio.ch.NioSocketImpl.connect(NioSocketImpl.java:567)\\n\\tat java.base/java.net.SocksSocketImpl.connect(SocksSocketImpl.java:327)\\n\\tat java.base/java.net.Socket.connect(Socket.java:751)\\n\\tat org.postgresql.core.PGStream.createSocket(PGStream.java:243)\\n\\tat org.postgresql.core.PGStream.<init>(PGStream.java:98)\\n\\tat org.postgresql.core.v3.ConnectionFactoryImpl.tryConnect(ConnectionFactoryImpl.java:132)\\n\\tat org.postgresql.core.v3.ConnectionFactoryImpl.openConnectionImpl(ConnectionFactoryImpl.java:258)\\n\\t... 22 more\\n\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"source\"}},\"cdc-demo-sink\":{\"info\":{\"config\":{\"connection.password\":\"demo\",\"connection.url\":\"jdbc:postgresql://cdc-postgres-target:5432/demo?currentSchema=bookings\",\"connection.username\":\"demo\",\"connector.class\":\"io.debezium.connector.jdbc.JdbcSinkConnector\",\"delete.enabled\":\"true\",\"field.exclude.list\":\"coordinates\",\"insert.mode\":\"upsert\",\"key.converter\":\"org.apache.kafka.connect.json.JsonConverter\",\"key.converter.schemas.enable\":\"true\",\"name\":\"cdc-demo-sink\",\"primary.key.mode\":\"record_key\",\"quote.identifiers\":\"true\",\"schema.evolution\":\"basic\",\"table.name.format\":\"${topic}\",\"tasks.max\":\"1\",\"topics.regex\":\"demo\\\\.(?!.*\\\\.routes$).*\",\"transforms\":\"unwrap,route\",\"transforms.route.regex\":\".*\\\\.([^.]+)$\",\"transforms.route.replacement\":\"$1\",\"transforms.route.type\":\"org.apache.kafka.connect.transforms.RegexRouter\",\"transforms.unwrap.delete.handling.mode\":\"rewrite\",\"transforms.unwrap.drop.tombstones\":\"false\",\"transforms.unwrap.type\":\"io.debezium.transforms.ExtractNewRecordState\",\"value.converter\":\"org.apache.kafka.connect.json.JsonConverter\",\"value.converter.schemas.enable\":\"true\"},\"name\":\"cdc-demo-sink\",\"tasks\":[{\"connector\":\"cdc-demo-sink\",\"task\":0}],\"type\":\"sink\"},\"status\":{\"connector\":{\"state\":\"RUNNING\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-demo-sink\",\"tasks\":[{\"id\":0,\"state\":\"FAILED\",\"trace\":\"org.hibernate.service.spi.ServiceException: Unable to create requested service [org.hibernate.engine.jdbc.env.spi.JdbcEnvironment] due to: Unable to determine Dialect without JDBC metadata (please set 'jakarta.persistence.jdbc.url' for common cases or 'hibernate.dialect' when a custom Dialect implementation must be provided)\\n\\tat org.hibernate.service.internal.AbstractServiceRegistryImpl.createService(AbstractServiceRegistryImpl.java:276)\\n\\tat org.hibernate.service.internal.AbstractServiceRegistryImpl.initializeService(AbstractServiceRegistryImpl.java:238)\\n\\tat org.hibernate.service.internal.AbstractServiceRegistryImpl.getService(AbstractServiceRegistryImpl.java:215)\\n\\tat org.hibernate.boot.model.relational.Database.<init>(Database.java:45)\\n\\tat org.hibernate.boot.internal.InFlightMetadataCollectorImpl.getDatabase(InFlightMetadataCollectorImpl.java:223)\\n\\tat org.hibernate.boot.internal.InFlightMetadataCollectorImpl.<init>(InFlightMetadataCollectorImpl.java:191)\\n\\tat org.hibernate.boot.model.process.spi.MetadataBuildingProcess.complete(MetadataBuildingProcess.java:170)\\n\\tat org.hibernate.boot.model.process.spi.MetadataBuildingProcess.build(MetadataBuildingProcess.java:129)\\n\\tat org.hibernate.boot.internal.MetadataBuilderImpl.build(MetadataBuilderImpl.java:449)\\n\\tat org.hibernate.boot.internal.MetadataBuilderImpl.build(MetadataBuilderImpl.java:101)\\n\\tat org.hibernate.cfg.Configuration.buildSessionFactory(Configuration.java:910)\\n\\tat org.hibernate.cfg.Configuration.buildSessionFactory(Configuration.java:960)\\n\\tat io.debezium.connector.jdbc.JdbcSinkConnectorTask.start(JdbcSinkConnectorTask.java:75)\\n\\tat org.apache.kafka.connect.runtime.WorkerSinkTask.initializeAndStart(WorkerSinkTask.java:329)\\n\\tat org.apache.kafka.connect.runtime.WorkerTask.doRun(WorkerTask.java:202)\\n\\tat org.apache.kafka.connect.runtime.WorkerTask.run(WorkerTask.java:259)\\n\\tat org.apache.kafka.connect.runtime.isolation.Plugins.lambda$withClassLoader$1(Plugins.java:237)\\n\\tat java.base/java.util.concurrent.Executors$RunnableAdapter.call(Executors.java:572)\\n\\tat java.base/java.util.concurrent.FutureTask.run(FutureTask.java:317)\\n\\tat java.base/java.util.concurrent.ThreadPoolExecutor.runWorker(ThreadPoolExecutor.java:1144)\\n\\tat java.base/java.util.concurrent.ThreadPoolExecutor$Worker.run(ThreadPoolExecutor.java:642)\\n\\tat java.base/java.lang.Thread.run(Thread.java:1583)\\nCaused by: org.hibernate.HibernateException: Unable to determine Dialect without JDBC metadata (please set 'jakarta.persistence.jdbc.url' for common cases or 'hibernate.dialect' when a custom Dialect implementation must be provided)\\n\\tat org.hibernate.engine.jdbc.dialect.internal.DialectFactoryImpl.determineDialect(DialectFactoryImpl.java:191)\\n\\tat org.hibernate.engine.jdbc.dialect.internal.DialectFactoryImpl.buildDialect(DialectFactoryImpl.java:87)\\n\\tat org.hibernate.engine.jdbc.env.internal.JdbcEnvironmentInitiator.getJdbcEnvironmentWithDefaults(JdbcEnvironmentInitiator.java:143)\\n\\tat org.hibernate.engine.jdbc.env.internal.JdbcEnvironmentInitiator.getJdbcEnvironmentUsingJdbcMetadata(JdbcEnvironmentInitiator.java:348)\\n\\tat org.hibernate.engine.jdbc.env.internal.JdbcEnvironmentInitiator.initiateService(JdbcEnvironmentInitiator.java:107)\\n\\tat org.hibernate.engine.jdbc.env.internal.JdbcEnvironmentInitiator.initiateService(JdbcEnvironmentInitiator.java:68)\\n\\tat org.hibernate.boot.registry.internal.StandardServiceRegistryImpl.initiateService(StandardServiceRegistryImpl.java:130)\\n\\tat org.hibernate.service.internal.AbstractServiceRegistryImpl.createService(AbstractServiceRegistryImpl.java:263)\\n\\t... 21 more\\n\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"sink\"}},\"cdc-draft---oracle\":{\"info\":{\"config\":{\"batch.max.rows\":\"500\",\"connection.password\":\"bq256crm\",\"connection.url\":\"jdbc:oracle:thin:@//61.91.6.88:1521/pandora\",\"connection.user\":\"crm\",\"connector.class\":\"io.confluent.connect.jdbc.JdbcSourceConnector\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"mode\":\"timestamp\",\"name\":\"cdc-draft---oracle\",\"numeric.mapping\":\"best_fit\",\"poll.interval.ms\":\"60000\",\"schema.pattern\":\"PANDORA\",\"table.include.list\":\"PANDORA\\\\.PM_PRODUCT\",\"tasks.max\":\"1\",\"timestamp.columns.mapping\":\".*:[UPDATE_DATE]\",\"topic.creation.default.cleanup.policy\":\"delete\",\"topic.creation.default.partitions\":\"1\",\"topic.creation.default.replication.factor\":\"1\",\"topic.creation.default.retention.ms\":\"604800000\",\"topic.creation.default.segment.bytes\":\"134217728\",\"topic.prefix\":\"draft___oracle.\",\"transforms\":\"lowercaseFields\",\"transforms.lowercaseFields.renames\":\"ACC_CODE:acc_code,BRANCH_NO:branch_no,BRANCH_PO:branch_po,COMPONENT_FLAG:component_flag,COST_AVG:cost_avg,COST_FIFOTOTAL:cost_fifototal,COST_LASTED:cost_lasted,COST_STD:cost_std,COST_TOTAL:cost_total,DECIMAL_INPUT:decimal_input,EDIT_DESC_INV:edit_desc_inv,EDIT_DESC_PO:edit_desc_po,EDIT_PRICE:edit_price,HIREPRICE_TYPE:hireprice_type,HIRE_CODE:hire_code,HIRE_STATUS:hire_status,INPUT_PRICEZERO:input_pricezero,INSERT_BY:insert_by,INSERT_DATE:insert_date,INSERT_TIME:insert_time,LOCATION_CODE:location_code,PACK_RATIO:pack_ratio,PACK_RECEIVE:pack_receive,PART_DISCODE:part_discode,PART_SEARCH:part_search,PO_REFMINMAX:po_refminmax,PROCOLOR_CODE:procolor_code,PROGROUP_CODE:progroup_code,PROSTEP_CODE:prostep_code,PROSUBGROUP_CODE:prosubgroup_code,PROTYPE_CODE:protype_code,PROUNIT_BUY:prounit_buy,PROUNIT_SALE:prounit_sale,PROUNIT_STORE:prounit_store,PRO_AGE:pro_age,PRO_BARCODE:pro_barcode,PRO_BRAND:pro_brand,PRO_BUY:pro_buy,PRO_CANCEL:pro_cancel,PRO_CANCELDATE:pro_canceldate,PRO_CODE:pro_code,PRO_FACTCODE:pro_factcode,PRO_LOT:pro_lot,PRO_MARGIN:pro_margin,PRO_MODEL:pro_model,PRO_MODEL_MK:pro_model_mk,PRO_MSTATUS:pro_mstatus,PRO_NAME:pro_name,PRO_NAME_E:pro_name_e,PRO_PCARMODEL:pro_pcarmodel,PRO_REFOEPART:pro_refoepart,PRO_REM:pro_rem,PRO_REPRE:pro_repre,PRO_SALE:pro_sale,PRO_SALENOJOB:pro_salenojob,PRO_SIZE:pro_size,PRO_SSTATUS:pro_sstatus,PRO_STOCK:pro_stock,PRO_SUB_TYPE:pro_sub_type,PRO_TYPE:pro_type,PRO_TYPE_GROUP:pro_type_group,PRO_USEBRANCH:pro_usebranch,PRO_WEIGHT:pro_weight,PURCHASE_MIN:purchase_min,PURCHASE_MIN_UNIT:purchase_min_unit,QC_STATUS:qc_status,QTY_BACKORD:qty_backord,QTY_INTRANSIT:qty_intransit,QTY_MAX:qty_max,QTY_MIN:qty_min,QTY_ONACC:qty_onacc,QTY_ONBORROW:qty_onborrow,QTY_ONCLAIM:qty_onclaim,QTY_ONDEFECT:qty_ondefect,QTY_ONHAND:qty_onhand,QTY_ONORDER:qty_onorder,QTY_ONPRODUCTION:qty_onproduction,QTY_ONREMAIN:qty_onremain,QTY_ONREQUEST:qty_onrequest,QTY_ONRESERVE:qty_onreserve,QTY_ONTRANSFER:qty_ontransfer,QTY_ORD:qty_ord,QTY_SAFETYSTOCK:qty_safetystock,REF_PART:ref_part,START_STOCK:start_stock,UPDATE_BY:update_by,UPDATE_DATE:update_date,UPDATE_TIME:update_time,VEN_CODE:ven_code,WEIGHT_TOTAL:weight_total\",\"transforms.lowercaseFields.type\":\"org.apache.kafka.connect.transforms.ReplaceField$Value\",\"validate.non.null\":\"true\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-draft---oracle\",\"tasks\":[{\"connector\":\"cdc-draft---oracle\",\"task\":0}],\"type\":\"source\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-draft---oracle\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"source\"}},\"cdc-draft---oracle-sink\":{\"info\":{\"config\":{\"connection.password\":\"demo\",\"connection.url\":\"jdbc:postgresql://host.docker.internal:25433/demo?currentSchema=PANDORA\",\"connection.username\":\"demo\",\"connector.class\":\"io.debezium.connector.jdbc.JdbcSinkConnector\",\"delete.enabled\":\"false\",\"insert.mode\":\"upsert\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"name\":\"cdc-draft---oracle-sink\",\"primary.key.fields\":\"branch_no,pro_code\",\"primary.key.mode\":\"record_value\",\"quote.identifiers\":\"true\",\"schema.evolution\":\"basic\",\"table.name.format\":\"${topic}\",\"tasks.max\":\"1\",\"topics.regex\":\"draft___oracle\\\\.(PM_PRODUCT)\",\"transforms\":\"route\",\"transforms.route.regex\":\".*\\\\.([^.]+)$\",\"transforms.route.replacement\":\"$1\",\"transforms.route.type\":\"org.apache.kafka.connect.transforms.RegexRouter\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-draft---oracle-sink\",\"tasks\":[{\"connector\":\"cdc-draft---oracle-sink\",\"task\":0}],\"type\":\"sink\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-draft---oracle-sink\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"sink\"}},\"cdc-pw-mvxizvb1-e2e\":{\"info\":{\"config\":{\"connector.class\":\"io.debezium.connector.postgresql.PostgresConnector\",\"database.dbname\":\"demo\",\"database.hostname\":\"host.docker.internal\",\"database.password\":\"demo\",\"database.port\":\"25432\",\"database.user\":\"demo\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"name\":\"cdc-pw-mvxizvb1-e2e\",\"plugin.name\":\"pgoutput\",\"publication.autocreate.mode\":\"disabled\",\"publication.name\":\"dbz_publication\",\"schema.include.list\":\"bookings\",\"slot.name\":\"cdc_pw_mvxizvb1_e2e\",\"snapshot.mode\":\"initial_only\",\"table.include.list\":\"bookings.airplanes_data\",\"tasks.max\":\"1\",\"topic.creation.default.cleanup.policy\":\"delete\",\"topic.creation.default.partitions\":\"1\",\"topic.creation.default.replication.factor\":\"1\",\"topic.creation.default.retention.ms\":\"604800000\",\"topic.creation.default.segment.bytes\":\"134217728\",\"topic.prefix\":\"pw_mvxizvb1_e2e\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-pw-mvxizvb1-e2e\",\"tasks\":[{\"connector\":\"cdc-pw-mvxizvb1-e2e\",\"task\":0}],\"type\":\"source\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-pw-mvxizvb1-e2e\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"source\"}},\"cdc-pw-mvxizvb1-e2e-sink\":{\"info\":{\"config\":{\"connection.password\":\"demo\",\"connection.url\":\"jdbc:postgresql://host.docker.internal:25433/demo?currentSchema=pw_mvxizvb1_e2e\",\"connection.username\":\"demo\",\"connector.class\":\"io.debezium.connector.jdbc.JdbcSinkConnector\",\"delete.enabled\":\"true\",\"field.exclude.list\":\"coordinates\",\"insert.mode\":\"upsert\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"name\":\"cdc-pw-mvxizvb1-e2e-sink\",\"predicates\":\"p0\",\"predicates.p0.pattern\":\".*bookings\\\\.airplanes_data$\",\"predicates.p0.type\":\"org.apache.kafka.connect.transforms.predicates.TopicNameMatches\",\"primary.key.mode\":\"record_key\",\"quote.identifiers\":\"true\",\"schema.evolution\":\"basic\",\"table.name.format\":\"${topic}\",\"tasks.max\":\"1\",\"topics.regex\":\"pw_mvxizvb1_e2e\\\\.(bookings\\\\.airplanes_data)\",\"transforms\":\"unwrap,map0,route\",\"transforms.map0.predicate\":\"p0\",\"transforms.map0.regex\":\".*\",\"transforms.map0.replacement\":\"airplanes_data\",\"transforms.map0.type\":\"org.apache.kafka.connect.transforms.RegexRouter\",\"transforms.route.regex\":\".*\\\\.([^.]+)$\",\"transforms.route.replacement\":\"$1\",\"transforms.route.type\":\"org.apache.kafka.connect.transforms.RegexRouter\",\"transforms.unwrap.delete.handling.mode\":\"rewrite\",\"transforms.unwrap.drop.tombstones\":\"false\",\"transforms.unwrap.type\":\"io.debezium.transforms.ExtractNewRecordState\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-pw-mvxizvb1-e2e-sink\",\"tasks\":[{\"connector\":\"cdc-pw-mvxizvb1-e2e-sink\",\"task\":0}],\"type\":\"sink\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-pw-mvxizvb1-e2e-sink\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"sink\"}},\"cdc-test01\":{\"info\":{\"config\":{\"batch.max.rows\":\"500\",\"connection.password\":\"password\",\"connection.url\":\"jdbc:oracle:thin:@//vpn.patanayont.com:61521/PRODORCL\",\"connection.user\":\"bcs\",\"connector.class\":\"io.confluent.connect.jdbc.JdbcSourceConnector\",\"errors.retry.delay.max.ms\":\"5000\",\"errors.retry.timeout\":\"60000\",\"incrementing.column.mapping\":\".*:ID\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"mode\":\"incrementing\",\"name\":\"cdc-test01\",\"numeric.mapping\":\"best_fit\",\"poll.interval.ms\":\"10000\",\"table.include.list\":\"(?:[^.]+\\\\.)?BCS\\\\.HYPER_CDC_CHANGE_LOG\",\"tasks.max\":\"1\",\"topic.creation.default.cleanup.policy\":\"delete\",\"topic.creation.default.partitions\":\"1\",\"topic.creation.default.replication.factor\":\"1\",\"topic.creation.default.retention.ms\":\"604800000\",\"topic.creation.default.segment.bytes\":\"134217728\",\"topic.prefix\":\"test01_\",\"transforms\":\"lowercaseFields,changelogTopic\",\"transforms.changelogTopic.regex\":\".*\",\"transforms.changelogTopic.replacement\":\"test01_hyper_cdc.change_log\",\"transforms.changelogTopic.type\":\"org.apache.kafka.connect.transforms.RegexRouter\",\"transforms.lowercaseFields.renames\":\"ID:id,SOURCE_SCHEMA:source_schema,SOURCE_TABLE:source_table,OP:op,PK:pk,ROW_DATA:row_data,CHANGED_AT:changed_at\",\"transforms.lowercaseFields.type\":\"org.apache.kafka.connect.transforms.ReplaceField$Value\",\"validate.non.null\":\"false\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-test01\",\"tasks\":[{\"connector\":\"cdc-test01\",\"task\":0}],\"type\":\"source\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-test01\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"source\"}},\"cdc-test01-sink\":{\"info\":{\"config\":{\"connection.password\":\"demo\",\"connection.url\":\"jdbc:postgresql://host.docker.internal:25433/demo?currentSchema=hyper_cdc\",\"connection.username\":\"demo\",\"connector.class\":\"io.debezium.connector.jdbc.JdbcSinkConnector\",\"delete.enabled\":\"false\",\"errors.deadletterqueue.context.headers.enable\":\"true\",\"errors.deadletterqueue.topic.name\":\"cdc-test01-sink-dlq\",\"errors.deadletterqueue.topic.replication.factor\":\"1\",\"errors.retry.delay.max.ms\":\"5000\",\"errors.retry.timeout\":\"60000\",\"errors.tolerance\":\"all\",\"insert.mode\":\"upsert\",\"key.converter\":\"io.confluent.connect.avro.AvroConverter\",\"key.converter.auto.register.schemas\":\"true\",\"key.converter.enhanced.avro.schema.support\":\"true\",\"key.converter.schema.registry.url\":\"http://cdc-message:8081\",\"name\":\"cdc-test01-sink\",\"primary.key.fields\":\"id\",\"primary.key.mode\":\"record_value\",\"quote.identifiers\":\"true\",\"schema.evolution\":\"none\",\"table.name.format\":\"inbox\",\"tasks.max\":\"1\",\"topics\":\"test01_hyper_cdc.change_log\",\"value.converter\":\"io.confluent.connect.avro.AvroConverter\",\"value.converter.auto.register.schemas\":\"true\",\"value.converter.enhanced.avro.schema.support\":\"true\",\"value.converter.schema.registry.url\":\"http://cdc-message:8081\"},\"name\":\"cdc-test01-sink\",\"tasks\":[{\"connector\":\"cdc-test01-sink\",\"task\":0}],\"type\":\"sink\"},\"status\":{\"connector\":{\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"},\"name\":\"cdc-test01-sink\",\"tasks\":[{\"id\":0,\"state\":\"PAUSED\",\"worker_id\":\"172.18.0.3:8083\"}],\"type\":\"sink\"}}},\"timestamp\":\"2026-11-01T07:46:00.921964671Z\"}"
```

# Test source

```ts
  276 |     });
  277 |   });
  278 | 
  279 |   test("sync_once start creates RUNNING source + sink connectors", async ({ request }) => {
  280 |     test.skip(!demo, DEMO_UNREACHABLE);
  281 |     sourceCount = (
  282 |       await json(
  283 |         await request.post(`${API}/compare/table/count`, {
  284 |           data: { connection: compareConnection(db.sourcePort), schema: db.schema, table: db.smallTable },
  285 |         })
  286 |       )
  287 |     ).count;
  288 |     expect(sourceCount).toBeGreaterThan(0);
  289 | 
  290 |     profile = await createProfile(request, `${RUN_PREFIX}-e2e`, {
  291 |       source: sourceConfig({ capture_mode: "sync_once" }),
  292 |       flow: {
  293 |         transforms: [],
  294 |         schedule: { delivery_mode: "realtime" },
  295 |         mappings: [
  296 |           {
  297 |             source_schema: db.schema,
  298 |             source_table: db.smallTable,
  299 |             target_schema: targetSchema,
  300 |             target_table: db.smallTable,
  301 |             // identical=true means "same names as the source"; a remap must say false.
  302 |             identical: false,
  303 |             columns: [],
  304 |           },
  305 |         ],
  306 |       },
  307 |     });
  308 |     const started = await request.post(`${API}/profiles/${profile.id}/actions/start`);
  309 |     await expectStatus(started, 200);
  310 |     expect((await json(started)).profile.status).toBe("running");
  311 | 
  312 |     const inv = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
  313 |     sourceConnector = inv.profile.source_connector;
  314 |     sinkConnector = inv.profile.sink_connector;
  315 |     expect(sourceConnector).toBeTruthy();
  316 |     expect(sinkConnector).toBeTruthy();
  317 | 
  318 |     const src = await connectorConfig(request, sourceConnector);
  319 |     slot = src?.["slot.name"] ?? "";
  320 |     topicPrefix = src?.["topic.prefix"] ?? "";
  321 |     expect(topicPrefix).toBeTruthy();
  322 | 
  323 |     await waitFor(
  324 |       async () => {
  325 |         const fresh = await json(await request.get(`${API}/profiles/${profile.id}/investigate`));
  326 |         const states = ["source", "sink"].map((k) => fresh.connectors?.[k]?.status?.connector?.state);
  327 |         return states.every((s) => s === "RUNNING");
  328 |       },
  329 |       60_000,
  330 |       "source and sink connectors RUNNING"
  331 |     );
  332 |   });
  333 | 
  334 |   test("capture: the source topic receives every row of the table", async ({ request }) => {
  335 |     test.skip(!profile, "previous step did not start the pipeline");
  336 |     const topicName = `${topicPrefix}.${db.schema}.${db.smallTable}`;
  337 |     const count = await waitFor(
  338 |       async () => {
  339 |         const body = await json(await request.get(`${API}/topics?search=${topicName}`));
  340 |         const t = body.items.find((x: any) => x.name === topicName);
  341 |         return t && t.messages_count >= sourceCount ? t.messages_count : undefined;
  342 |       },
  343 |       90_000,
  344 |       `${topicName} to hold ${sourceCount} messages`
  345 |     );
  346 |     expect(count).toBe(sourceCount);
  347 |   });
  348 | 
  349 |   test("KNOWN BUG: sink writes to the mapped target schema (schema-qualified table)", async ({ request }) => {
  350 |     test.fail(true, "sink uses currentSchema + unqualified table.name.format; Debezium resolves an existing same-named table in another schema — see docs/testing/api-test-plan.md");
  351 |     test.skip(!profile, "previous step did not start the pipeline");
  352 |     const cfg = await connectorConfig(request, sinkConnector);
  353 |     expect(cfg?.["table.name.format"] ?? "").toMatch(new RegExp(`^"?${targetSchema}"?\\.`));
  354 |   });
  355 | 
  356 |   test("KNOWN BUG: sink sees new topics without the 5-minute regex metadata delay", async ({ request }) => {
  357 |     test.fail(true, "sink subscribes via topics.regex before topics exist; first data waits for metadata.max.age.ms (5 min) — see docs/testing/api-test-plan.md");
  358 |     test.skip(!profile, "previous step did not start the pipeline");
  359 |     const cfg = (await connectorConfig(request, sinkConnector)) ?? {};
  360 |     const fastRefresh = Number(cfg["consumer.override.metadata.max.age.ms"] ?? Infinity) <= 30_000;
  361 |     expect(Boolean(cfg["topics"]) || fastRefresh).toBe(true);
  362 |   });
  363 | 
  364 |   test.fixme("rows land in the mapped target schema", async () => {
  365 |     // Enable once the two sink bugs above are fixed: poll
  366 |     // POST /compare/table/count on { targetSchema, smallTable } until it equals sourceCount.
  367 |   });
  368 | 
  369 |   test("KNOWN BUG: deleting the profile removes its connectors", async ({ request }) => {
  370 |     test.fail(true, "delete_profile only pauses connectors when ORCH_STOP_DELETES_CONNECTOR=false (default), orphaning them and the replication slot — see docs/testing/api-test-plan.md");
  371 |     test.skip(!profile, "previous step did not start the pipeline");
  372 |     await expectStatus(await request.delete(`${API}/profiles/${profile.id}`), 204);
  373 |     created.delete(profile.id);
  374 |     await new Promise((r) => setTimeout(r, 3000));
  375 |     const listing = await (await request.get("/connect/connectors")).text();
> 376 |     expect(listing).not.toContain(`"${sourceConnector}"`);
      |                         ^ Error: expect(received).not.toContain(expected) // indexOf
  377 |     expect(listing).not.toContain(`"${sinkConnector}"`);
  378 |   });
  379 | });
  380 | 
  381 | test.describe("profiles — test initial sync stops afterwards (ORCH_TEST_E2E=true)", () => {
  382 |   test.skip(!e2eEnabled, "set ORCH_TEST_E2E=true to start pipelines and write to the demo target");
  383 |   test.setTimeout(180_000);
  384 | 
  385 |   const schema = `${RUN_PREFIX.replace(/-/g, "_")}_ts`;
  386 |   const name = `${RUN_PREFIX}-testsync`;
  387 |   const target = compareConnection(db.targetPort);
  388 |   let profile: any;
  389 | 
  390 |   const ddl = (request: any, statement: string) =>
  391 |     request.post(`${API}/compare/target/exec-ddl`, { data: { target, statement } });
  392 |   const ownConnectors = async (request: any) =>
  393 |     ((await (await request.get("/connect/connectors")).text()).match(new RegExp(`"cdc-${name}[^"]*"`, "g")) ?? []).map(
  394 |       (n: string) => n.replace(/"/g, "")
  395 |     );
  396 |   const targetCount = async (request: any) =>
  397 |     (
  398 |       await json(
  399 |         await request.post(`${API}/compare/table/count`, { data: { connection: target, schema, table: "flights" } })
  400 |       )
  401 |     ).count as number;
  402 | 
  403 |   test.beforeAll(async ({ request }) => {
  404 |     test.skip(!(await demoSourceReachable(request)), DEMO_UNREACHABLE);
  405 |     // Target table with an extra `__deleted` column, as left by a Debezium sink — the copy
  406 |     // must still succeed (it copies shared columns by name).
  407 |     await expectStatus(
  408 |       await ddl(
  409 |         request,
  410 |         `CREATE SCHEMA IF NOT EXISTS "${schema}"; ` +
  411 |           `CREATE TABLE IF NOT EXISTS "${schema}".flights (LIKE bookings.flights INCLUDING DEFAULTS); ` +
  412 |           `ALTER TABLE "${schema}".flights ADD COLUMN IF NOT EXISTS __deleted text`
  413 |       ),
  414 |       200
  415 |     );
  416 |     profile = await createProfile(request, name, {
  417 |       source: sourceConfig({
  418 |         capture_mode: "incremental",
  419 |         tables: [`${db.schema}.flights`],
  420 |         incrementing_column: "flight_id",
  421 |         upsert_key_columns: ["flight_id"],
  422 |       }),
  423 |       flow: {
  424 |         transforms: [],
  425 |         schedule: { delivery_mode: "micro_batch", interval_seconds: 60 },
  426 |         mappings: [
  427 |           {
  428 |             source_schema: db.schema,
  429 |             source_table: "flights",
  430 |             target_schema: schema,
  431 |             target_table: "flights",
  432 |             identical: false,
  433 |             columns: [],
  434 |             pk_columns: ["flight_id"],
  435 |           },
  436 |         ],
  437 |       },
  438 |     });
  439 |   });
  440 | 
  441 |   test.afterAll(async ({ request }) => {
  442 |     if (profile) await request.delete(`${API}/profiles/${profile.id}`).catch(() => undefined);
  443 |     created.delete(profile?.id);
  444 |     for (const c of await ownConnectors(request)) await request.delete(`/connect/connectors/${c}`);
  445 |     const prefix = name.replace(/-/g, "_");
  446 |     const topics = await json(await request.get(`${API}/topics?search=${prefix}&page_size=50`)).catch(() => ({ items: [] }));
  447 |     for (const t of topics.items ?? []) await request.delete(`${API}/topics/${t.name}`);
  448 |     await ddl(request, `DROP SCHEMA IF EXISTS "${schema}" CASCADE`);
  449 |   });
  450 | 
  451 |   test("row_limit is rejected for logical (WAL) pipelines", async ({ request }) => {
  452 |     const logical = await createProfile(request, `${RUN_PREFIX}-testsync-logical`);
  453 |     const res = await request.post(`${API}/profiles/${logical.id}/actions/start?row_limit=5`);
  454 |     await expectError(res, 400, /row_limit applies only/);
  455 |   });
  456 | 
  457 |   test("copies the sample, then ends stopped with no connectors left", async ({ request }) => {
  458 |     const res = await request.post(`${API}/profiles/${profile.id}/actions/start?row_limit=5&clear_target=true`);
  459 |     await expectStatus(res, 200);
  460 |     const after = (await json(res)).profile;
  461 |     expect(after.status).toBe("stopped");
  462 |     expect(after.last_error).toBeNull();
  463 |     const last = after.history.at(-1);
  464 |     expect(last.event).toBe("profile_test_initial_sync");
  465 |     expect(last.detail).toContain(`${schema}.flights=5`);
  466 |     expect(last.detail).toMatch(/not a production sync/);
  467 | 
  468 |     expect(await ownConnectors(request)).toEqual([]);
  469 |     expect(await targetCount(request)).toBe(5);
  470 | 
  471 |     // Nothing keeps syncing: still stopped with exactly the sample a little later.
  472 |     await new Promise((r) => setTimeout(r, 10_000));
  473 |     expect((await json(await request.get(`${API}/profiles/${profile.id}`))).profile.status).toBe("stopped");
  474 |     expect(await targetCount(request)).toBe(5);
  475 | 
  476 |     const audit = await json(await request.get(`${API}/audit/events?profile_id=${profile.id}&limit=5`));
```