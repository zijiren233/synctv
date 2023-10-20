package conf

type DatabaseType string

const (
	DatabaseTypeSqlite3  DatabaseType = "sqlite3"
	DatabaseTypeMysql    DatabaseType = "mysql"
	DatabaseTypePostgres DatabaseType = "postgres"
)

type DatabaseConfig struct {
	Type     DatabaseType `yaml:"type" lc:"default: sqlite3" hc:"support sqlite3, mysql, postgres" env:"DATABASE_TYPE"`
	Host     string       `yaml:"host" hc:"when type is not sqlite3, and port is 0, it will use unix socket file" env:"DATABASE_HOST"`
	Port     uint16       `yaml:"port" env:"DATABASE_PORT"`
	User     string       `yaml:"user" env:"DATABASE_USER"`
	Password string       `yaml:"password" env:"DATABASE_PASSWORD"`
	DBName   string       `yaml:"db_name" lc:"default: synctv" hc:"when type is sqlite3, it will use sqlite db file or memory" env:"DATABASE_DB_NAME"`
	SslMode  string       `yaml:"ssl_mode" env:"DATABASE_SSL_MODE"`

	CustomDSN string `yaml:"custom_dsn" hc:"when not empty, it will ignore other config" env:"DATABASE_CUSTOM_DSN"`

	MaxIdleConns    int    `yaml:"max_idle_conns" lc:"default: 4" hc:"the maximum number of connections in the idle connection pool." env:"DATABASE_MAX_IDLE_CONNS"`
	MaxOpenConns    int    `yaml:"max_open_conns" lc:"default: 64" hc:"the maximum number of open connections to the database." env:"DATABASE_MAX_OPEN_CONNS"`
	ConnMaxLifetime string `yaml:"conn_max_lifetime" lc:"default: 1h" hc:"maximum amount of time a connection may be reused." env:"DATABASE_CONN_MAX_LIFETIME"`
}

func DefaultDatabaseConfig() DatabaseConfig {
	return DatabaseConfig{
		Type:    DatabaseTypeSqlite3,
		Host:    "",
		DBName:  "synctv",
		SslMode: "disable",

		MaxIdleConns:    4,
		MaxOpenConns:    64,
		ConnMaxLifetime: "1h",
	}
}
