use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Deserialization error: {context}")]
    Deserialization { context: String },

    #[error("Authentication error: {0}")]
    Authentication(String),

    #[error("Authorization error: {0}")]
    Authorization(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Optimistic lock conflict")]
    OptimisticLockConflict,
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            // Map "no rows" to NotFound
            sqlx::Error::RowNotFound => Error::NotFound("Resource not found".to_string()),
            // Map unique constraint violations to AlreadyExists
            sqlx::Error::Database(db_err) => {
                let code = db_err.code().unwrap_or_default();
                match code.as_ref() {
                    // PostgreSQL unique_violation
                    "23505" => {
                        let detail = db_err.message().to_string();
                        if detail.contains("username") {
                            Error::AlreadyExists("Username already taken".to_string())
                        } else if detail.contains("email") {
                            Error::AlreadyExists("Email already registered".to_string())
                        } else {
                            Error::AlreadyExists("Resource already exists".to_string())
                        }
                    }
                    // PostgreSQL foreign_key_violation
                    "23503" => Error::NotFound("Referenced resource not found".to_string()),
                    // PostgreSQL check_violation
                    "23514" => Error::InvalidInput("Constraint check failed".to_string()),
                    // PostgreSQL not_null_violation
                    "23502" => Error::InvalidInput("Required field is missing".to_string()),
                    _ => Error::Database(err),
                }
            }
            _ => Error::Database(err),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
