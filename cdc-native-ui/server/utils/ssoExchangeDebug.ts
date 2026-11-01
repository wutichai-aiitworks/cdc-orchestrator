/** null = follow env; true/false = in-memory override for this process */
let override: boolean | null = null

export function setSsoExchangeDebug(on: boolean | null): void {
  override = on
}

export function isSsoExchangeDebug(): boolean {
  if (override !== null) return override
  return process.env.SSO_EXCHANGE_DEBUG === "true"
}

export function getSsoExchangeDebugState() {
  return {
    enabled: isSsoExchangeDebug(),
    override,
    env: process.env.SSO_EXCHANGE_DEBUG === "true",
  }
}
