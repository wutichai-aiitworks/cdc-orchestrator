"""Deployment mode (docs/patterns/deployment-mode). Keep APP_MODE; rename APP_REQUIRE_* and the retired names to this repo's.

Same contract as the Rust/TS versions: APP_MODE = Production (default) | Maintenance | Enhanced.
"""
from __future__ import annotations

import enum
import logging
import os
import re

log = logging.getLogger(__name__)


class AppMode(str, enum.Enum):
    PRODUCTION = "production"
    MAINTENANCE = "maintenance"
    ENHANCED = "enhanced"

    @classmethod
    def parse(cls, raw: str | None) -> "AppMode | None":
        v = (raw or "").strip().lower()
        if re.fullmatch(r"prod(uction)?", v):
            return cls.PRODUCTION
        if re.fullmatch(r"maint(enance|ainance)?", v):
            return cls.MAINTENANCE
        if re.fullmatch(r"enhance(d|ments?)?", v):
            return cls.ENHANCED
        return None

    @classmethod
    def resolve(cls, mode: str | None, legacy_env: str | None = None) -> "AppMode":
        """APP_MODE, else the retired APP_ENV (dev -> Enhanced), else Production."""
        m = cls.parse(mode)
        if m:
            return m
        return cls.ENHANCED if re.fullmatch(r"dev(elopment)?", (legacy_env or "").strip().lower()) else cls.PRODUCTION

    @property
    def hardened_by_default(self) -> bool:
        return self is AppMode.PRODUCTION


def env_truthy(key: str) -> bool | None:
    """Unset or empty -> None (compose `${VAR:-}` is "", meaning default, not False)."""
    v = os.environ.get(key, "").strip().lower()
    return None if not v else v in {"1", "true", "yes", "on"}


def load_mode() -> tuple[AppMode, dict[str, bool]]:
    legacy_env = os.environ.get("APP_ENV", "").strip() or None  # retired variable — rename to this repo's
    mode = AppMode.resolve(os.environ.get("APP_MODE"), legacy_env)
    def flag(key: str) -> bool:
        v = env_truthy(key)
        return mode.hardened_by_default if v is None else v

    flags = {"require_auth": flag("APP_REQUIRE_AUTH"), "require_persistence": flag("APP_REQUIRE_PERSISTENCE")}
    log.info("starting mode=%s %s", mode.value, flags)
    if legacy_env:
        log.warning("APP_ENV is retired: use APP_MODE=Production|Maintenance|Enhanced only")
    return mode, flags
