// Lets `node --test` load the Nuxt utils, which import siblings without a `.ts`
// extension (Vite/Bundler resolution). Node strips TypeScript types natively.
import { registerHooks } from "node:module"

registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context)
    } catch (err) {
      if (specifier.startsWith(".") && !/\.[cm]?[jt]s$/.test(specifier)) {
        return next(`${specifier}.ts`, context)
      }
      throw err
    }
  },
})
