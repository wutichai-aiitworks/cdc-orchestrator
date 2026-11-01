import { formatApiError } from "~/utils/apiError"

export const useCdcCapability = () => {
  const data = ref<any>(null)
  const loading = ref(false)
  const error = ref("")

  const refresh = async () => {
    loading.value = true
    error.value = ""
    try {
      data.value = await $fetch("/api/cdc/capability")
    } catch (e: any) {
      error.value = formatApiError(e, "Failed to fetch capability")
    } finally {
      loading.value = false
    }
  }

  return { data, loading, error, refresh }
}
