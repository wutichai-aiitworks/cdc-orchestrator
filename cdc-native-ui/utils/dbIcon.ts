const DB_ICONS: Record<string, string> = {
  postgres:   "/images/drivers/postgresql_icon_big.png",
  mysql:      "/images/drivers/mysql_icon_big.png",
  mariadb:    "/images/drivers/mariadb_icon_big.png",
  sqlserver:  "/images/drivers/mssql_icon_big.png",
  oracle:     "/images/drivers/oracle_icon_big.png",
  db2:        "/images/drivers/db2_i_icon_big.png",
  informix:   "/images/drivers/informix_icon_big.png",
  sqlite:     "/images/drivers/sqlite_icon_big.png",
}

const FALLBACK = "/images/drivers/database_icon@2x.png"

export function dbIconPath(type: string | null | undefined): string {
  return DB_ICONS[(type ?? "").toLowerCase()] ?? FALLBACK
}

export interface DbTypeOption {
  value: string
  label: string
  port: string
  icon: string
}

export const DB_TYPE_OPTIONS: DbTypeOption[] = [
  { value: "postgres",   label: "PostgreSQL",  port: "5432",  icon: DB_ICONS.postgres },
  { value: "mysql",      label: "MySQL",        port: "3306",  icon: DB_ICONS.mysql },
  { value: "mariadb",    label: "MariaDB",      port: "3306",  icon: DB_ICONS.mariadb },
  { value: "sqlserver",  label: "SQL Server",   port: "1433",  icon: DB_ICONS.sqlserver },
  { value: "oracle",     label: "Oracle",       port: "1521",  icon: DB_ICONS.oracle },
  { value: "db2",        label: "IBM DB2",      port: "50000", icon: DB_ICONS.db2 },
  { value: "informix",   label: "Informix",     port: "9088",  icon: DB_ICONS.informix },
  { value: "sqlite",     label: "SQLite",       port: "",      icon: DB_ICONS.sqlite },
]
