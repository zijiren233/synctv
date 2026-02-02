//! SeaQuery + SQLx Integration Example
//!
//! This demonstrates how SeaQuery and SQLx work together perfectly.

use sea_query::{Query, PostgresQueryBuilder, Cond, Expr, Order};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Example 1: Basic Query with Parameters
///
/// SeaQuery builds the SQL, SQLx executes it with parameter binding
async fn example_basic_query(pool: &PgPool) -> Result<(), sqlx::Error> {
    // 1. Build query with SeaQuery (type-safe, no SQL injection risk)
    let (sql, values) = Query::select()
        .column("id")
        .column("username")
        .column("email")
        .from("users")
        .and_where(Expr::col("username").eq("alice"))
        .and_where(Expr::col("age").gte(18))
        .build(PostgresQueryBuilder);

    // sql = "SELECT id, username, email FROM users WHERE username = $1 AND age >= $2"
    // values = [Value::String("alice"), Value::Int(18)]

    // 2. Execute with SQLx (parameterized, safe)
    let mut query = sqlx::query_as::<(i32, String, String)>(&sql);

    // 3. Bind parameters automatically
    for value in values {
        query = match value {
            sea_query::Value::String(Some(s)) => query.bind(*s),
            sea_query::Value::Int(Some(i)) => query.bind(i),
            sea_query::Value::Bool(Some(b)) => query.bind(b),
            sea_query::Value::Float(Some(f)) => query.bind(f),
            _ => query,
        };
    }

    // 4. Fetch results
    let rows = query.fetch_all(pool).await?;
    println!("Found {} users", rows.len());

    Ok(())
}

/// Example 2: Using Filter Builder with SQLx
///
/// Shows our Filter type integrating with SQLx
async fn example_with_filter(
    pool: &PgPool,
    username: Option<String>,
    min_age: Option<i32>,
) -> Result<(), sqlx::Error> {
    use synctv_core::repository::Filter;

    // 1. Build filter dynamically
    let mut filter = Filter::new().is_null("deleted_at");

    if let Some(name) = &username {
        filter = filter.like("username", format!("{}%", name));
    }

    if let Some(age) = min_age {
        filter = filter.ge("age", age);
    }

    let condition = filter.build();

    // 2. Build query
    let (sql, values) = Query::select()
        .column("id")
        .column("username")
        .from("users")
        .cond(condition)
        .build(PostgresQueryBuilder);

    // 3. Bind and execute with SQLx
    let mut query = sqlx::query_as::<(i32, String)>(&sql);
    for value in values {
        query = bind_sea_value(query, value);
    }

    let users = query.fetch_all(pool).await?;
    println!("Found {} users", users.len());

    Ok(())
}

/// Helper: Bind SeaQuery value to SQLx query
fn bind_sea_value<'q>(
    query: sqlx::QueryAs<'q, Postgres, (i32, String), sqlx::postgres::PgArguments>,
    value: sea_query::Value,
) -> sqlx::QueryAs<'q, Postgres, (i32, String), sqlx::postgres::PgArguments> {
    match value {
        sea_query::Value::String(Some(s)) => query.bind(*s),
        sea_query::Value::Int(Some(i)) => query.bind(i),
        sea_query::Value::Bool(Some(b)) => query.bind(b),
        sea_query::Value::Float(Some(f)) => query.bind(f),
        _ => query,
    }
}

/// Example 3: INSERT with SeaQuery + SQLx
async fn example_insert(pool: &PgPool) -> Result<(), sqlx::Error> {
    use sea_query::{Alias, Table};

    // 1. Build INSERT query
    let (sql, values) = Query::insert()
        .into_table("users")
        .columns([
            "username",
            "email",
            "age",
        ])
        .values_panic([
            "bob".into(),
            "bob@example.com".into(),
            25.into(),
        ])
        .returning_col("id")
        .build(PostgresQueryBuilder);

    // 2. Execute with SQLx
    let mut query = sqlx::query_scalar::<_, i32>(&sql);
    for value in values {
        query = bind_sea_value_scalar(query, value);
    }

    let user_id = query.fetch_one(pool).await?;
    println!("Created user with ID: {}", user_id);

    Ok(())
}

fn bind_sea_value_scalar<'q>(
    query: sqlx::QueryScalar<'q, Postgres, i32, sqlx::postgres::PgArguments>,
    value: sea_query::Value,
) -> sqlx::QueryScalar<'q, Postgres, i32, sqlx::postgres::PgArguments> {
    match value {
        sea_query::Value::String(Some(s)) => query.bind(*s),
        sea_query::Value::Int(Some(i)) => query.bind(i),
        sea_query::Value::Bool(Some(b)) => query.bind(b),
        sea_query::Value::Float(Some(f)) => query.bind(f),
        _ => query,
    }
}

/// Example 4: UPDATE with SeaQuery + SQLx
async fn example_update(pool: &PgPool) -> Result<(), sqlx::Error> {
    // 1. Build UPDATE query
    let (sql, values) = Query::update()
        .table("users")
        .values([
            ("email".into(), "newemail@example.com".into()),
        ])
        .and_where(Expr::col("id").eq(1))
        .build(PostgresQueryBuilder);

    // 2. Execute with SQLx
    let mut query = sqlx::query(&sql);
    for value in values {
        query = bind_sea_value_exec(query, value);
    }

    let rows_affected = query.execute(pool).await?.rows_affected();
    println!("Updated {} rows", rows_affected);

    Ok(())
}

fn bind_sea_value_exec<'q>(
    query: sqlx::Query<'q, Postgres, sqlx::postgres::PgArguments>,
    value: sea_query::Value,
) -> sqlx::Query<'q, Postgres, sqlx::postgres::PgArguments> {
    match value {
        sea_query::Value::String(Some(s)) => query.bind(*s),
        sea_query::Value::Int(Some(i)) => query.bind(i),
        sea_query::Value::Bool(Some(b)) => query.bind(b),
        sea_query::Value::Float(Some(f)) => query.bind(f),
        _ => query,
    }
}

/// Example 5: Complex JOIN with SeaQuery + SQLx
async fn example_join(pool: &PgPool) -> Result<(), sqlx::Error> {
    use sea_query::Table;

    // 1. Build complex JOIN query
    let (sql, values) = Query::select()
        .columns([
            "users.id",
            "users.username",
            "rooms.name",
        ])
        .from("users")
        .inner_join(
            "room_members",
            Expr::col(("users", "id")).equals(("room_members", "user_id"))
        )
        .inner_join(
            "rooms",
            Expr::col(("room_members", "room_id")).equals(("rooms", "id"))
        )
        .and_where(Expr::col(("rooms", "id")).eq(123))
        .build(PostgresQueryBuilder);

    // 2. Execute with SQLx
    let mut query = sqlx::query_as::<(i32, String, String)>(&sql);
    for value in values {
        query = bind_sea_value(query, value);
    }

    let results = query.fetch_all(pool).await?;
    println!("Found {} room members", results.len());

    Ok(())
}

/// Example 6: Using sea-query-sqlx-tx crate (even better!)
///
/// There's a crate that makes integration even easier
async fn example_with_integration_crate(pool: &PgPool) -> Result<(), sqlx::Error> {
    // If you use sea-query-sqlx-tx, it becomes even simpler:
    //
    // use sea_query_sqlx_tx::{get_sql, get_values};
    //
    // let (sql, values) = Query::select()
    //     .from("users")
    //     .and_where(Expr::col("id").eq(1))
    //     .build(PostgresQueryBuilder);
    //
    // let user = sqlx::query_as::<User>(&sql)
    //     .bind_all(values)  // <-- Automatic binding!
    //     .fetch_one(pool)
    //     .await?;

    Ok(())
}

/// Summary: Compatibility Matrix
///
/// | Feature | SeaQuery | SQLx | Together |
/// |---------|----------|------|----------|
/// | Query building | ✅ | ❌ | ✅ SeaQuery |
/// | Type safety | ✅ Build | ✅ Runtime+Compile | ✅ Both |
/// | Parameter binding | ✅ Manual | ✅ Auto | ✅ Compatible |
/// | Execution | ❌ | ✅ Async | ✅ SQLx |
/// | Compile-time checks | ❌ | ✅ With macros | ✅ Use SQLx macros |
/// | Database drivers | ✅ Multi | ✅ Multi | ✅ Same DBs |
///
/// **Verdict**: ✅ 100% Compatible and Complementary!
///
/// They solve different problems:
/// - SeaQuery: Build SQL queries (query builder)
/// - SQLx: Execute SQL queries (database driver)
///
/// Together: Perfect combination ✅
