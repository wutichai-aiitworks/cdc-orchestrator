// Deployment mode (docs/patterns/deployment-mode). Keep APP_MODE; rename APP_REQUIRE_* and the retired names to this repo's.
package config

import (
	"log/slog"
	"os"
	"regexp"
	"strings"
)

type AppMode string

const (
	Production  AppMode = "production"
	Maintenance AppMode = "maintenance"
	Enhanced    AppMode = "enhanced"
)

var (
	reProd  = regexp.MustCompile(`^prod(uction)?$`)
	reMaint = regexp.MustCompile(`^maint(enance|ainance)?$`)
	reEnh   = regexp.MustCompile(`^enhance(d|ments?)?$`)
	reDev   = regexp.MustCompile(`^dev(elopment)?$`)
)

// ParseMode: case-insensitive, common spellings; "" for unknown.
func ParseMode(raw string) AppMode {
	v := strings.ToLower(strings.TrimSpace(raw))
	switch {
	case reProd.MatchString(v):
		return Production
	case reMaint.MatchString(v):
		return Maintenance
	case reEnh.MatchString(v):
		return Enhanced
	}
	return ""
}

// ResolveMode: APP_MODE, else the retired APP_ENV (dev → Enhanced), else Production.
func ResolveMode(mode, legacyEnv string) AppMode {
	if m := ParseMode(mode); m != "" {
		return m
	}
	if reDev.MatchString(strings.ToLower(strings.TrimSpace(legacyEnv))) {
		return Enhanced
	}
	return Production
}

func (m AppMode) HardenedByDefault() bool { return m == Production }

// EnvTruthy: unset or empty → (false, false) meaning "use the default".
func EnvTruthy(key string) (value bool, set bool) {
	v := strings.ToLower(strings.TrimSpace(os.Getenv(key)))
	if v == "" {
		return false, false
	}
	return v == "1" || v == "true" || v == "yes" || v == "on", true
}

func flag(key string, def bool) bool {
	if v, ok := EnvTruthy(key); ok {
		return v
	}
	return def
}

type ModeConfig struct {
	Mode               AppMode
	RequireAuth        bool
	RequirePersistence bool
}

func LoadMode() ModeConfig {
	mode := ResolveMode(os.Getenv("APP_MODE"), os.Getenv("APP_ENV"))
	c := ModeConfig{Mode: mode, RequireAuth: flag("APP_REQUIRE_AUTH", mode.HardenedByDefault()), RequirePersistence: flag("APP_REQUIRE_PERSISTENCE", mode.HardenedByDefault())}
	slog.Info("starting", "mode", mode, "require_auth", c.RequireAuth)
	if strings.TrimSpace(os.Getenv("APP_ENV")) != "" {
		slog.Warn("APP_ENV is retired: use APP_MODE=Production|Maintenance|Enhanced only")
	}
	return c
}
