/**
 * Extra breadcrumb crumbs a page appends after its menu trail, e.g. the pipeline
 * name and "Edit" on /pipelines. The page owns the value and clears it on unmount.
 */
export const usePageCrumbs = () => useState<string[]>("page-crumbs", () => [])
