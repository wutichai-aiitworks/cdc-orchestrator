export const getSessionConfig = () => {
  const config = useRuntimeConfig()
  return {
    password: config.sessionSecret,
    maxAge: 60 * 60 * 8,
    name: "cdc-session",
    cookie: {
      sameSite: "strict" as const,
      httpOnly: true,
      secure: config.sessionCookieSecure as boolean,
    },
  }
}
