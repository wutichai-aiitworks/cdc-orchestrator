-- Target tables for CDC profile "test01" (Oracle BCS -> PostgreSQL demo).
--
-- Naming rules that must be followed for trigger apply to work:
--   * Schema and table names are UPPERCASE and quoted, because the profile has no
--     table mapping, so target names are copied verbatim from the Oracle source.
--   * Column names are lowercase and unquoted. hyper_cdc.apply_inbox() lowercases
--     every JSON key before calling jsonb_populate_record(), which matches keys to
--     column names case-sensitively.
--   * Non-PK columns are left nullable even though Oracle marks them NOT NULL, so a
--     partial change payload cannot abort the apply trigger.

CREATE SCHEMA IF NOT EXISTS "BCS";

CREATE TABLE IF NOT EXISTS "BCS"."G_BOOKS" (
    book_id bigint       NOT NULL,
    isbn    varchar(1000),
    title   varchar(1000),
    authors varchar(1000),
    CONSTRAINT g_books_pk PRIMARY KEY (book_id)
);

CREATE TABLE IF NOT EXISTS "BCS"."G_BOOK_AUTHORS" (
    book_id   bigint NOT NULL,
    author_id bigint NOT NULL,
    CONSTRAINT g_book_authors_pk PRIMARY KEY (book_id, author_id)
);

CREATE TABLE IF NOT EXISTS "BCS"."G_CART" (
    customer_id   bigint NOT NULL,
    cart_sequence bigint NOT NULL,
    book_id       bigint,
    quantity      bigint,
    date_added    timestamp,
    CONSTRAINT g_cart_pk PRIMARY KEY (customer_id, cart_sequence)
);

CREATE TABLE IF NOT EXISTS "BCS"."G_CUSTOMERS" (
    customer_id  bigint NOT NULL,
    firstname    varchar(60),
    surname      varchar(60),
    dob          timestamp,
    gender       varchar(1),
    emailaddress varchar(400),
    password     varchar(400),
    CONSTRAINT g_customers_pk PRIMARY KEY (customer_id)
);

CREATE TABLE IF NOT EXISTS "BCS"."G_LINE_ITEMS" (
    order_id bigint NOT NULL,
    line_id  bigint NOT NULL,
    book_id  bigint,
    quantity bigint,
    CONSTRAINT g_line_items_pk PRIMARY KEY (order_id, line_id)
);

CREATE TABLE IF NOT EXISTS "BCS"."G_ORDERS" (
    order_id    bigint NOT NULL,
    order_date  timestamp,
    customer_id bigint,
    CONSTRAINT g_orders_pk PRIMARY KEY (order_id)
);
