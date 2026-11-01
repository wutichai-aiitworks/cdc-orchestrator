type ToastType = "success" | "error" | "info" | "warning"

interface Toast {
  id: string
  message: string
  type: ToastType
  duration: number
}

const toasts = ref<Toast[]>([])

export const useToast = () => {
  const remove = (id: string) => {
    toasts.value = toasts.value.filter((toast) => toast.id !== id)
  }

  const add = (message: string, type: ToastType = "info", duration = 4000) => {
    const id = crypto.randomUUID()
    toasts.value.push({ id, message, type, duration })
    setTimeout(() => remove(id), duration)
  }

  return {
    toasts,
    remove,
    success: (message: string) => add(message, "success"),
    error: (message: string) => add(message, "error"),
    info: (message: string) => add(message, "info"),
    warning: (message: string) => add(message, "warning"),
  }
}
