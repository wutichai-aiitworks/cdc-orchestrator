# Production preflight fragment. Needs pass/warn/fail helpers and env_value KEY (reads the env file).
app_mode=$(env_value APP_MODE)
mode_lc=$(printf '%s' "$app_mode" | tr '[:upper:]' '[:lower:]')
case "$mode_lc" in
  production|prod|maintenance|maintainance|maint|enhanced|enhance|enhancement) pass "APP_MODE=$app_mode" ;;
  *) fail "APP_MODE must be Production, Maintenance or Enhanced, got '${app_mode:-unset}'" ;;
esac
if [ -n "$(env_value APP_ENV)" ]; then warn "APP_ENV is retired (APP_MODE replaces it); remove it"; fi
require_true() {
  local value
  value=$(env_value "$1")
  if [ "$value" = true ]; then
    pass "$1=true"
  elif [ -z "$value" ] && { [ "$mode_lc" = production ] || [ "$mode_lc" = prod ]; }; then
    pass "$1 defaults to true (APP_MODE=Production)"
  else
    fail "$1 must be true (or APP_MODE=Production with it unset), got '${value:-unset}'"
  fi
}
require_true APP_REQUIRE_AUTH
require_true APP_REQUIRE_PERSISTENCE
require_true APP_STRICT_PERSISTENCE
