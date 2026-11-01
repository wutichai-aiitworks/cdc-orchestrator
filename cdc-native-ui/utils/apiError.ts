const readableString = (value: unknown) =>
  typeof value === "string" && value.trim() && value !== "true" && value !== "false"
    ? value.trim()
    : ""

export const formatApiError = (error: any, fallback: string) => {
  const status = error?.statusCode || error?.status || error?.response?.status
  const message =
    readableString(error?.data?.message) ||
    readableString(error?.data?.statusMessage) ||
    readableString(error?.data?.error) ||
    readableString(error?.statusMessage) ||
    readableString(error?.message) ||
    fallback

  return status ? `[${status}] ${message}` : message
}
