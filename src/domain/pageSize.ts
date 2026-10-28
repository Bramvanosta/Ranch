/** The page sizes a job list offers; the user picks one in Settings › General. */
export const PAGE_SIZES = [25, 50, 100, 250] as const
export type PageSize = (typeof PAGE_SIZES)[number]
export const DEFAULT_PAGE_SIZE: PageSize = 50

export function isPageSize(value: unknown): value is PageSize {
  return PAGE_SIZES.includes(value as PageSize)
}
