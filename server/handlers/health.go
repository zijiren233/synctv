package handlers

import (
	"net/http"

	"github.com/gin-gonic/gin"
	"github.com/synctv-org/synctv/internal/db"
	"github.com/synctv-org/synctv/server/model"
)

// Health returns a simple health check endpoint
// Returns 200 OK if the service is running
func Health(ctx *gin.Context) {
	ctx.JSON(http.StatusOK, gin.H{
		"status": "ok",
	})
}

// Ready returns readiness check endpoint
// Checks if all dependencies (database) are accessible
func Ready(ctx *gin.Context) {
	// Check database connectivity
	sqlDB, err := db.DB().DB()
	if err != nil {
		ctx.JSON(http.StatusServiceUnavailable, model.NewAPIErrorResp(err))
		return
	}

	if err := sqlDB.Ping(); err != nil {
		ctx.JSON(http.StatusServiceUnavailable, model.NewAPIErrorResp(err))
		return
	}

	// Check database connection pool health
	stats := sqlDB.Stats()
	if stats.OpenConnections == 0 {
		ctx.JSON(http.StatusServiceUnavailable, gin.H{
			"status": "unhealthy",
			"reason": "no database connections available",
		})
		return
	}

	ctx.JSON(http.StatusOK, gin.H{
		"status": "ready",
		"database": gin.H{
			"open_connections": stats.OpenConnections,
			"in_use":           stats.InUse,
			"idle":             stats.Idle,
		},
	})
}

// Live returns liveness check endpoint (Kubernetes liveness probe)
// This is simpler than readiness - just checks if the process is alive
func Live(ctx *gin.Context) {
	ctx.JSON(http.StatusOK, gin.H{
		"status": "alive",
	})
}
