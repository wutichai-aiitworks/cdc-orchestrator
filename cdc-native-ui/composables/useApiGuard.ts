export function useApiGuard() {
  const router = useRouter()

  function onError(err: any) {
    const status = err?.statusCode || err?.status || err?.response?.status
    if (status === 401) {
      const next = router.currentRoute.value?.fullPath || "/"
      navigateTo(`/auth/login?next=${encodeURIComponent(next)}`)
    }
  }

  return { onError }
}
