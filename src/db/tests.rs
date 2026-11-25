use super::*;
use sqlx::Row;

#[test]
fn rewrites_numbered_placeholders_for_postgres() {
    assert_eq!(
        rewrite_placeholders("SELECT * FROM t WHERE a = ?1 AND b = ?2"),
        "SELECT * FROM t WHERE a = $1 AND b = $2"
    );
    // Repeated placeholders keep the same number.
    assert_eq!(
        rewrite_placeholders("SELECT $x FROM t WHERE a = ?1 OR b = ?1"),
        "SELECT $x FROM t WHERE a = $1 OR b = $1"
    );
    // Out-of-order numbered placeholders are preserved.
    assert_eq!(
        rewrite_placeholders("UPDATE t SET name = ?2 WHERE id = ?1"),
        "UPDATE t SET name = $2 WHERE id = $1"
    );
    // Literal question marks inside strings are untouched.
    assert_eq!(
        rewrite_placeholders("SELECT * FROM t WHERE note = 'why?' AND id = ?1"),
        "SELECT * FROM t WHERE note = 'why?' AND id = $1"
    );
}

#[test]
fn builds_sqlite_urls_from_paths() {
    assert_eq!(
        sqlite_url_from_path(r"C:\data\test.db"),
        "sqlite:///C:/data/test.db?mode=rwc"
    );
    assert_eq!(
        sqlite_url_from_path("data/test.db"),
        "sqlite://data/test.db?mode=rwc"
    );
    assert_eq!(
        sqlite_url_from_path("/var/lib/tsp.db"),
        "sqlite:///var/lib/tsp.db?mode=rwc"
    );
}

#[test]
fn normalizes_mariadb_urls() {
    assert_eq!(
        mysql_url_from("mariadb://user:pw@db:3306/tsp"),
        "mysql://user:pw@db:3306/tsp"
    );
    assert_eq!(
        mysql_url_from("mysql://user:pw@db:3306/tsp"),
        "mysql://user:pw@db:3306/tsp"
    );
}

#[test]
fn rewrites_numbered_placeholders_for_mysql() {
    // Repeated and out-of-order markers are rewritten to `?` while the
    // occurrence list records which bound value each marker needs.
    let (sql, occurrences) =
        mysql_placeholders("UPDATE t SET name = ?2 WHERE id = ?1 OR parent = ?1");
    assert_eq!(sql, "UPDATE t SET name = ? WHERE id = ? OR parent = ?");
    assert_eq!(occurrences, vec![1, 0, 0]);

    // A literal question mark inside a string must not become a marker.
    let (sql, occurrences) = mysql_placeholders("SELECT * FROM t WHERE note = 'why?' AND id = ?1");
    assert_eq!(sql, "SELECT * FROM t WHERE note = 'why?' AND id = ?");
    assert_eq!(occurrences, vec![0]);

    // A repeated trailing value (`VALUES (... ?6, ?6, ...)`) must expand.
    let (sql, occurrences) = mysql_placeholders("VALUES (?1, ?2, ?6, ?6)");
    assert_eq!(sql, "VALUES (?, ?, ?, ?)");
    assert_eq!(occurrences, vec![0, 1, 5, 5]);
}

#[test]
fn splits_mysql_schema_into_statements() {
    let statements = split_sql_statements(MYSQL_SCHEMA);
    assert_eq!(statements.len(), 14);
    assert!(statements
        .iter()
        .all(|stmt| { stmt.starts_with("CREATE TABLE") || stmt.starts_with("CREATE INDEX") }));
    // Semicolons inside quoted defaults must not split a statement.
    assert_eq!(
        split_sql_statements("SELECT ';' AS semi; SELECT 1").len(),
        2
    );
}

#[tokio::test]
async fn sqlite_binds_repeated_and_out_of_order_placeholders() {
    let db = Db::in_memory().await.expect("in-memory db");
    let row = db
        .query("SELECT ?2 AS second, ?1 AS first, ?1 AS first_again")
        .bind("one")
        .bind("two")
        .fetch_one(db.pool())
        .await
        .expect("row");
    let second: String = row.try_get(0).expect("second");
    let first: String = row.try_get(1).expect("first");
    let first_again: String = row.try_get(2).expect("first_again");
    assert_eq!(second, "two");
    assert_eq!(first, "one");
    assert_eq!(first_again, "one");
}

/// Optional end-to-end smoke test. It is skipped unless a disposable MySQL
/// server is configured, e.g.
/// `TSP_TEST_MYSQL_URL=mysql://root:pw@127.0.0.1:3306/tsp_test cargo test --lib`.
#[tokio::test]
async fn mysql_placeholder_expansion_when_configured() {
    let Ok(url) = std::env::var("TSP_TEST_MYSQL_URL") else {
        return;
    };
    let db = Db::open(url).await.expect("mysql database");
    db.ping().await.expect("ping");
    let mut tx = db.pool().begin().await.expect("transaction");
    db.query("CREATE TEMPORARY TABLE tsp_compat_test (id VARCHAR(32), a INTEGER, b VARCHAR(32))")
        .execute(&mut tx)
        .await
        .expect("create temporary table");
    db.query("INSERT INTO tsp_compat_test (id, a, b) VALUES (?1, ?2, ?1)")
        .bind("same")
        .bind(7_i64)
        .execute(&mut tx)
        .await
        .expect("insert repeated placeholder");
    let row = db
        .query("SELECT id, a, b FROM tsp_compat_test WHERE id = ?1 AND b = ?1")
        .bind("same")
        .fetch_one(&mut tx)
        .await
        .expect("select repeated placeholder");
    assert_eq!(row.try_get::<String, _>(0).unwrap(), "same");
    assert_eq!(row.try_get::<i64, _>(1).unwrap(), 7);
    assert_eq!(row.try_get::<String, _>(2).unwrap(), "same");
    tx.rollback().await.expect("rollback");
}

#[test]
fn normalizes_kingbase_urls_to_postgres_wire() {
    assert_eq!(
        kingbase_url_from("kingbase://tsp:pw@kes:54321/tsp?sslmode=disable"),
        "postgres://tsp:pw@kes:54321/tsp?sslmode=disable"
    );
    assert_eq!(
        kingbase_url_from("kingbase8://kes/tsp"),
        "postgres://kes/tsp"
    );
    assert_eq!(
        kingbase_url_from("kingbasees://kes:54321/tsp"),
        "postgres://kes:54321/tsp"
    );
}

#[tokio::test]
async fn sqlite_engine_name_is_sqlite() {
    let db = Db::in_memory().await.expect("db");
    assert_eq!(db.engine_name(), "sqlite");
    assert!(!db.is_kingbase());
}
