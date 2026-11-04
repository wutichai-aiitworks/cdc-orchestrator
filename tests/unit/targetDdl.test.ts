// node --test --import ./tests/unit/register-ts.mjs tests/unit/*.test.ts
import assert from "node:assert/strict"
import { test } from "node:test"

import {
  declaredSourceType,
  generateTargetDdl,
  type DdlSourceColumn,
  type DdlSourceIndex,
} from "../../kafbat-webui/utils/targetDdl.ts"

// Oracle source shaped like jdbc-admin output: bare TYPE_NAME + COLUMN_SIZE/DECIMAL_DIGITS.
const oracleColumns: DdlSourceColumn[] = [
  { name: "BRAND_ID", data_type: "NUMBER", size: 10, scale: 0, nullable: false, primary_key: true, pk_position: 1 },
  { name: "BRAND_CODE", data_type: "VARCHAR2", size: 20, nullable: false },
  { name: "BRAND_NAME", data_type: "NVARCHAR2", size: 200, nullable: true },
  { name: "PRICE", data_type: "NUMBER", size: 12, scale: 2, nullable: true, default_value: "0 " },
  { name: "QTY", data_type: "NUMBER", size: 0, scale: -127, nullable: true },
  { name: "CREATED_AT", data_type: "DATE", size: 7, nullable: true, default_value: "SYSDATE\n" },
  { name: "NOTE", data_type: "CLOB", nullable: true, default_value: "'n/a'" },
]
const oracleIndexes: DdlSourceIndex[] = [
  { name: "PK_PM_PROBRAND", unique: true, columns: ["BRAND_ID"] },
  { name: "UQ_PM_PROBRAND_CODE", unique: true, columns: ["BRAND_CODE"] },
  { name: "IX_PM_PROBRAND_NAME_PRICE", unique: false, columns: ["BRAND_NAME", "PRICE"], descending: ["PRICE"] },
  { name: "IX_PM_PROBRAND_UPPER", unique: false, columns: ["SYS_NC00008$"] },
]

const oracleInput = (targetDialect: string) => ({
  sourceDialect: "oracle",
  targetDialect,
  targetSchema: "PANDORA",
  targetTable: "PM_PROBRAND",
  columns: oracleColumns,
  primaryKey: ["BRAND_ID"],
  indexes: oracleIndexes,
})

test("declaredSourceType rebuilds lengths and precision from JDBC metadata", () => {
  assert.equal(declaredSourceType({ name: "a", data_type: "VARCHAR2", size: 50, nullable: true }), "VARCHAR2(50)")
  assert.equal(declaredSourceType({ name: "a", data_type: "NUMBER", size: 10, scale: 2, nullable: true }), "NUMBER(10,2)")
  assert.equal(declaredSourceType({ name: "a", data_type: "NUMBER", size: 0, scale: -127, nullable: true }), "NUMBER")
  assert.equal(declaredSourceType({ name: "a", data_type: "numeric", size: 131089, scale: 0, nullable: true }), "numeric")
  assert.equal(declaredSourceType({ name: "a", data_type: "varchar", size: 2147483647, nullable: true }), "varchar")
  assert.equal(declaredSourceType({ name: "a", data_type: "timestamp", size: 29, scale: 6, nullable: true }), "timestamp")
  assert.equal(declaredSourceType({ name: "a", data_type: "character varying(40)", size: 40, nullable: true }), "character varying(40)")
  assert.equal(declaredSourceType({ name: "a", data_type: "int identity", size: 10, nullable: false }), "int")
})

test("Oracle → Postgres: lowercase names, PK, unique and index, translated types and defaults", () => {
  const r = generateTargetDdl(oracleInput("postgres"))
  assert.equal(r.dialect, "postgres")
  assert.deepEqual(r.statements[0], 'CREATE SCHEMA IF NOT EXISTS "pandora"')
  const create = r.statements[1]
  assert.match(create, /^CREATE TABLE IF NOT EXISTS "pandora"\."pm_probrand" \(/)
  assert.match(create, /"brand_id" BIGINT NOT NULL/)
  assert.match(create, /"brand_code" VARCHAR\(20\) NOT NULL/)
  assert.match(create, /"brand_name" VARCHAR\(200\),/)
  assert.match(create, /"price" NUMERIC\(12,2\) DEFAULT 0,/)
  assert.match(create, /"qty" NUMERIC,/)
  assert.match(create, /"created_at" TIMESTAMP DEFAULT CURRENT_TIMESTAMP,/)
  assert.match(create, /"note" TEXT DEFAULT 'n\/a',/)
  assert.match(create, /PRIMARY KEY \("brand_id"\)\n\)$/)
  assert.deepEqual(r.statements.slice(2), [
    'CREATE UNIQUE INDEX IF NOT EXISTS "uq_pm_probrand_code" ON "pandora"."pm_probrand" ("brand_code")',
    'CREATE INDEX IF NOT EXISTS "ix_pm_probrand_name_price" ON "pandora"."pm_probrand" ("brand_name", "price" DESC)',
  ])
  assert.ok(r.notes.some((n) => n.includes("IX_PM_PROBRAND_UPPER") && n.includes("expression")))
  assert.ok(!r.statements.some((s) => s.includes("PK_PM_PROBRAND")), "PK index is not duplicated")
  assert.ok(r.script.includes(";\n\nCREATE TABLE"), "script terminates statements with ;")
})

test("Oracle → Oracle keeps source types and wraps each statement in an ORA-00955 guard", () => {
  const r = generateTargetDdl(oracleInput("oracle"))
  assert.equal(r.statements.length, 3)
  assert.match(r.statements[0], /^BEGIN\n {2}EXECUTE IMMEDIATE 'CREATE TABLE "PANDORA"\."PM_PROBRAND" \(/)
  assert.match(r.statements[0], /"BRAND_CODE" VARCHAR2\(20\) NOT NULL/)
  assert.match(r.statements[0], /"NOTE" CLOB DEFAULT ''n\/a''/, "quotes are doubled inside EXECUTE IMMEDIATE")
  assert.match(r.statements[0], /IF SQLCODE NOT IN \(-955\) THEN RAISE; END IF;\nEND;$/)
  assert.match(r.statements[2], /IF SQLCODE NOT IN \(-955, -1408\)/)
  assert.ok(r.script.includes("END;\n/"), "script uses / after PL/SQL blocks")
})

const pgColumns: DdlSourceColumn[] = [
  { name: "id", data_type: "bigint", nullable: false, primary_key: true, pk_position: 1, default_value: "nextval('shop.product_id_seq'::regclass)", auto_increment: true },
  { name: "sku", data_type: "character varying(40)", nullable: false },
  { name: "title", data_type: "text", nullable: true, default_value: "'untitled'::text" },
  { name: "active", data_type: "boolean", nullable: false, default_value: "true" },
  { name: "price", data_type: "numeric(10,2)", nullable: true },
  { name: "tags", data_type: "text[]", nullable: true },
  { name: "updated_at", data_type: "timestamp with time zone", nullable: true, default_value: "now()" },
]
const pgIndexes: DdlSourceIndex[] = [
  { name: "product_pkey", unique: true, columns: ["id"] },
  { name: "product_sku_key", unique: true, columns: ["sku"] },
  { name: "product_lower_title", unique: false, columns: [], expression: true },
  { name: "product_active_price", unique: false, columns: ["active", "price"], filter: "(active = true)" },
]
const pgInput = (targetDialect: string, extra: object = {}) => ({
  sourceDialect: "postgres",
  targetDialect,
  targetSchema: "shop",
  targetTable: "product",
  columns: pgColumns,
  indexes: pgIndexes,
  ...extra,
})

test("Postgres → Postgres drops sequences, keeps case and skips partial/expression indexes", () => {
  const r = generateTargetDdl(pgInput("postgres"))
  const create = r.statements[1]
  assert.match(create, /"id" bigint NOT NULL/)
  assert.ok(!create.includes("nextval"), "sequence default dropped")
  assert.match(create, /"title" text DEFAULT 'untitled'/)
  assert.match(create, /"active" boolean DEFAULT TRUE NOT NULL/)
  assert.match(create, /"tags" text\[\]/)
  assert.match(create, /"updated_at" timestamp with time zone DEFAULT CURRENT_TIMESTAMP/)
  assert.match(create, /PRIMARY KEY \("id"\)/, "PK comes from pk_position when primaryKey is omitted")
  assert.deepEqual(r.statements.slice(2), ['CREATE UNIQUE INDEX IF NOT EXISTS "product_sku_key" ON "shop"."product" ("sku")'])
  assert.ok(r.notes.some((n) => n.includes("product_active_price") && n.includes("partial")))
  assert.ok(r.notes.some((n) => n.includes("product_lower_title") && n.includes("expression")), "expression index noted, not dropped silently")
  assert.ok(r.notes.some((n) => n.includes("Identity/sequence dropped on id")))
})

test("Postgres → SQL Server uses brackets, OBJECT_ID guards and BIT defaults", () => {
  const r = generateTargetDdl(pgInput("sqlserver"))
  assert.equal(r.statements[0], "IF SCHEMA_ID('shop') IS NULL EXEC(N'CREATE SCHEMA [shop]')")
  assert.match(r.statements[1], /^IF OBJECT_ID\(N'\[shop\]\.\[product\]', N'U'\) IS NULL\nCREATE TABLE \[shop\]\.\[product\] \(/)
  assert.match(r.statements[1], /\[sku\] VARCHAR\(40\) NOT NULL/)
  assert.match(r.statements[1], /\[active\] BIT DEFAULT 1 NOT NULL/)
  assert.match(r.statements[2], /^IF NOT EXISTS \(SELECT 1 FROM sys\.indexes WHERE name = N'product_sku_key'/)
  assert.ok(r.notes.some((n) => n.includes('"text[]"')), "array type flagged for review")
})

test("Postgres → MySQL puts keys inline so CREATE TABLE IF NOT EXISTS stays idempotent", () => {
  const r = generateTargetDdl(pgInput("mysql"))
  assert.equal(r.statements.length, 1)
  assert.match(r.statements[0], /^CREATE TABLE IF NOT EXISTS `shop`\.`product` \(/)
  assert.match(r.statements[0], /UNIQUE KEY `product_sku_key` \(`sku`\)/)
  assert.match(r.statements[0], /`title` LONGTEXT DEFAULT 'untitled'/)
})

test("missing index metadata and missing PK are reported, not guessed", () => {
  const r = generateTargetDdl({
    sourceDialect: "postgres",
    targetDialect: "postgres",
    targetSchema: "",
    targetTable: "events",
    columns: [{ name: "payload", data_type: "jsonb", nullable: true }],
  })
  assert.equal(r.statements.length, 1)
  assert.equal(r.statements[0], 'CREATE TABLE IF NOT EXISTS "events" (\n  "payload" jsonb\n)')
  assert.ok(r.notes.some((n) => n.includes("no primary key")))
  assert.ok(r.notes.some((n) => n.includes("not reported")))
})

test("unsupported targets return no statements", () => {
  const r = generateTargetDdl({ ...pgInput("mongodb") })
  assert.equal(r.dialect, null)
  assert.deepEqual(r.statements, [])
})
