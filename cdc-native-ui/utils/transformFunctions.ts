export type ArgType = "string" | "number" | "select" | "boolean"

export type TransformArg = {
  key: string
  label: string
  type: ArgType
  options?: string[]
  default?: string | number | boolean
  placeholder?: string
  required?: boolean
}

export type TransformFn = {
  id: string
  label: string
  category: "String" | "Numeric" | "Date/Time" | "Conditional" | "Type Cast" | "Security"
  description: string
  /** Minimum number of source columns required (default 1) */
  minSources?: number
  /** Maximum number of source columns allowed (default 1, use 99 for unlimited) */
  maxSources?: number
  args: TransformArg[]
  example: string
}

export const TRANSFORM_FUNCTIONS: TransformFn[] = [
  // ── String ───────────────────────────────────────────────────────────────
  {
    id: "concat",
    label: "Concat",
    category: "String",
    description: "Join two or more source columns into one string with a separator.",
    minSources: 2,
    maxSources: 99,
    args: [
      { key: "separator", label: "Separator", type: "string", default: " ", placeholder: "space, comma, pipe …" },
    ],
    example: "first_name + last_name → full_name (separator: ' ')",
  },
  {
    id: "coalesce",
    label: "Coalesce",
    category: "String",
    description: "Return the first non-null value across the source columns.",
    minSources: 2,
    maxSources: 99,
    args: [],
    example: "email_work, email_personal → contact_email",
  },
  {
    id: "upper",
    label: "Uppercase",
    category: "String",
    description: "Convert the source column value to uppercase.",
    args: [],
    example: "name → 'JOHN DOE'",
  },
  {
    id: "lower",
    label: "Lowercase",
    category: "String",
    description: "Convert the source column value to lowercase.",
    args: [],
    example: "status → 'active'",
  },
  {
    id: "trim",
    label: "Trim",
    category: "String",
    description: "Strip leading and trailing whitespace from the source value.",
    args: [],
    example: "'  hello  ' → 'hello'",
  },
  {
    id: "truncate",
    label: "Truncate",
    category: "String",
    description: "Shorten the string to at most N characters.",
    args: [
      { key: "length", label: "Max length", type: "number", default: 255, required: true },
    ],
    example: "long_description → short_description (255 chars)",
  },
  {
    id: "replace",
    label: "Replace",
    category: "String",
    description: "Replace all occurrences of a literal substring with another.",
    args: [
      { key: "from", label: "Find", type: "string", placeholder: "text to find", required: true },
      { key: "to", label: "Replace with", type: "string", placeholder: "replacement", required: true },
    ],
    example: "phone: '(555) 123-4567' → '5551234567' (remove chars)",
  },
  {
    id: "regexp_replace",
    label: "Regex Replace",
    category: "String",
    description: "Replace matches of a regular expression with a replacement string.",
    args: [
      { key: "pattern", label: "Pattern (regex)", type: "string", placeholder: "[^0-9]", required: true },
      { key: "replacement", label: "Replacement", type: "string", placeholder: "", default: "" },
      { key: "flags", label: "Flags", type: "string", placeholder: "gi", default: "g" },
    ],
    example: "phone → digits only: regexp_replace('[^0-9]', '')",
  },
  {
    id: "split_part",
    label: "Split Part",
    category: "String",
    description: "Split the value by a delimiter and return the Nth part (1-based).",
    args: [
      { key: "delimiter", label: "Delimiter", type: "string", placeholder: ",", required: true },
      { key: "part", label: "Part index (1-based)", type: "number", default: 1, required: true },
    ],
    example: "'John,Doe,30' → 'John' (delimiter: ',', part: 1)",
  },
  {
    id: "pad_left",
    label: "Pad Left",
    category: "String",
    description: "Left-pad the value to a total length with a fill character.",
    args: [
      { key: "length", label: "Total length", type: "number", default: 10, required: true },
      { key: "fill", label: "Fill character", type: "string", default: "0" },
    ],
    example: "id: '42' → '0000000042' (length: 10, fill: '0')",
  },

  // ── Numeric ──────────────────────────────────────────────────────────────
  {
    id: "add",
    label: "Add",
    category: "Numeric",
    description: "Sum two or more numeric source columns.",
    minSources: 2,
    maxSources: 99,
    args: [],
    example: "amount + tax → total",
  },
  {
    id: "subtract",
    label: "Subtract",
    category: "Numeric",
    description: "Subtract source[1..] from source[0].",
    minSources: 2,
    maxSources: 2,
    args: [],
    example: "gross - discount → net",
  },
  {
    id: "multiply",
    label: "Multiply",
    category: "Numeric",
    description: "Multiply all numeric source columns together.",
    minSources: 2,
    maxSources: 99,
    args: [],
    example: "unit_price × quantity → line_total",
  },
  {
    id: "divide",
    label: "Divide",
    category: "Numeric",
    description: "Divide source[0] by source[1].",
    minSources: 2,
    maxSources: 2,
    args: [],
    example: "total / count → average",
  },
  {
    id: "round",
    label: "Round",
    category: "Numeric",
    description: "Round to N decimal places.",
    args: [
      { key: "scale", label: "Decimal places", type: "number", default: 2 },
    ],
    example: "3.14159 → 3.14 (scale: 2)",
  },
  {
    id: "abs",
    label: "Absolute",
    category: "Numeric",
    description: "Return the absolute (non-negative) value.",
    args: [],
    example: "-42 → 42",
  },

  // ── Date / Time ──────────────────────────────────────────────────────────
  {
    id: "to_date",
    label: "Parse Date",
    category: "Date/Time",
    description: "Parse a string column as a date using a format pattern.",
    args: [
      {
        key: "format",
        label: "Format",
        type: "select",
        options: ["YYYY-MM-DD", "DD/MM/YYYY", "MM/DD/YYYY", "YYYY-MM-DD HH:mm:ss", "custom"],
        default: "YYYY-MM-DD",
        required: true,
      },
      { key: "custom_format", label: "Custom format", type: "string", placeholder: "DD-Mon-YYYY" },
    ],
    example: "'2024-01-15' → 2024-01-15 (date)",
  },
  {
    id: "to_timestamp",
    label: "Parse Timestamp",
    category: "Date/Time",
    description: "Parse a string column as a timestamp.",
    args: [
      { key: "format", label: "Format", type: "string", placeholder: "YYYY-MM-DD HH:mm:ss", default: "YYYY-MM-DD HH:mm:ss" },
      { key: "timezone", label: "Timezone", type: "string", placeholder: "UTC", default: "UTC" },
    ],
    example: "'2024-01-15 10:30:00' → timestamp",
  },
  {
    id: "date_trunc",
    label: "Date Truncate",
    category: "Date/Time",
    description: "Truncate a timestamp/date to the given precision.",
    args: [
      {
        key: "unit",
        label: "Truncate to",
        type: "select",
        options: ["year", "quarter", "month", "week", "day", "hour", "minute"],
        default: "day",
        required: true,
      },
    ],
    example: "2024-03-15 10:30 → 2024-03-15 00:00 (day)",
  },
  {
    id: "date_format",
    label: "Format Date",
    category: "Date/Time",
    description: "Format a date/timestamp as a string.",
    args: [
      { key: "format", label: "Output format", type: "string", placeholder: "YYYY-MM-DD", default: "YYYY-MM-DD", required: true },
    ],
    example: "timestamp → '2024-01-15'",
  },
  {
    id: "date_diff",
    label: "Date Diff",
    category: "Date/Time",
    description: "Calculate the difference between two date columns.",
    minSources: 2,
    maxSources: 2,
    args: [
      {
        key: "unit",
        label: "Unit",
        type: "select",
        options: ["days", "hours", "minutes", "months", "years"],
        default: "days",
        required: true,
      },
    ],
    example: "end_date - start_date → duration_days",
  },
  {
    id: "extract",
    label: "Extract Part",
    category: "Date/Time",
    description: "Extract a specific part from a date/timestamp.",
    args: [
      {
        key: "part",
        label: "Part",
        type: "select",
        options: ["year", "month", "day", "hour", "minute", "second", "week", "quarter", "dow"],
        default: "year",
        required: true,
      },
    ],
    example: "order_date → 2024 (year)",
  },
  {
    id: "now",
    label: "Current Timestamp",
    category: "Date/Time",
    description: "Replace the column value with the current timestamp at write time (audit column).",
    args: [],
    example: "→ 2024-01-15 10:30:00 UTC",
  },

  // ── Conditional ──────────────────────────────────────────────────────────
  {
    id: "if_null",
    label: "If Null",
    category: "Conditional",
    description: "Replace null values with a default value.",
    args: [
      { key: "default", label: "Default value", type: "string", placeholder: "0 or unknown", required: true },
    ],
    example: "discount → 0 when null",
  },
  {
    id: "if_empty",
    label: "If Empty",
    category: "Conditional",
    description: "Replace empty-string values with a default.",
    args: [
      { key: "default", label: "Default value", type: "string", placeholder: "N/A", required: true },
    ],
    example: "description → 'N/A' when empty string",
  },
  {
    id: "case_when",
    label: "Case/When",
    category: "Conditional",
    description: "Map input values to output values using a lookup table.",
    args: [
      { key: "mapping", label: "Value mappings (JSON)", type: "string", placeholder: '{"A":"Active","I":"Inactive"}', required: true },
      { key: "default", label: "Default (unmatched)", type: "string", placeholder: "Unknown" },
    ],
    example: "status_code 'A' → 'Active', 'I' → 'Inactive'",
  },
  {
    id: "boolean_to_int",
    label: "Boolean to Int",
    category: "Conditional",
    description: "Convert boolean true/false to 1/0.",
    args: [
      { key: "true_value", label: "True value", type: "string", default: "1" },
      { key: "false_value", label: "False value", type: "string", default: "0" },
    ],
    example: "is_active: true → 1",
  },

  // ── Type Cast ────────────────────────────────────────────────────────────
  {
    id: "to_string",
    label: "To String",
    category: "Type Cast",
    description: "Cast any value to a string (varchar).",
    args: [],
    example: "order_id: 42 → '42'",
  },
  {
    id: "to_integer",
    label: "To Integer",
    category: "Type Cast",
    description: "Parse the source value as an integer.",
    args: [
      { key: "on_error", label: "On parse error", type: "select", options: ["null", "zero", "error"], default: "null" },
    ],
    example: "'42' → 42",
  },
  {
    id: "to_decimal",
    label: "To Decimal",
    category: "Type Cast",
    description: "Parse the source value as a decimal number.",
    args: [
      { key: "scale", label: "Decimal places", type: "number", default: 2 },
      { key: "on_error", label: "On parse error", type: "select", options: ["null", "zero", "error"], default: "null" },
    ],
    example: "'3.14' → 3.14",
  },
  {
    id: "to_boolean",
    label: "To Boolean",
    category: "Type Cast",
    description: "Interpret the source value as a boolean.",
    args: [
      { key: "true_values", label: "True values (comma-sep)", type: "string", placeholder: "1,true,yes,Y", default: "1,true,yes,y" },
      { key: "false_values", label: "False values (comma-sep)", type: "string", placeholder: "0,false,no,N", default: "0,false,no,n" },
    ],
    example: "'yes' → true",
  },

  // ── Security / Privacy ───────────────────────────────────────────────────
  {
    id: "mask_pii",
    label: "Mask PII",
    category: "Security",
    description: "Partially mask sensitive values (shows first and last N characters).",
    args: [
      { key: "visible_start", label: "Visible start chars", type: "number", default: 2 },
      { key: "visible_end", label: "Visible end chars", type: "number", default: 2 },
      { key: "mask_char", label: "Mask character", type: "string", default: "*" },
    ],
    example: "email: 'john@example.com' → 'jo**@example.com'",
  },
  {
    id: "hash_md5",
    label: "Hash MD5",
    category: "Security",
    description: "Replace the value with its MD5 hex digest (non-reversible).",
    args: [],
    example: "password → 'd41d8cd98f00b204e9800998ecf8427e'",
  },
  {
    id: "hash_sha256",
    label: "Hash SHA-256",
    category: "Security",
    description: "Replace the value with its SHA-256 hex digest.",
    args: [
      { key: "salt", label: "Salt (optional)", type: "string", placeholder: "secret-salt" },
    ],
    example: "ssn → 'e3b0c44298fc…'",
  },
  {
    id: "redact",
    label: "Redact",
    category: "Security",
    description: "Replace the entire value with a fixed redaction string.",
    args: [
      { key: "replacement", label: "Replacement text", type: "string", default: "[REDACTED]" },
    ],
    example: "credit_card_number → '[REDACTED]'",
  },
]

export const TRANSFORM_BY_ID = new Map(TRANSFORM_FUNCTIONS.map((fn) => [fn.id, fn]))

export const TRANSFORM_CATEGORIES = [...new Set(TRANSFORM_FUNCTIONS.map((fn) => fn.category))]
