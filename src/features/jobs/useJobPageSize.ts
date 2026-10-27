import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback } from "react"

import { announceSettingsChange } from "@/lib/settingsSync"
import { commands } from "@/lib/tauri/bindings"

import { DEFAULT_PAGE_SIZE, isPageSize, type PageSize } from "./useJobs"

const KEY = "jobs.pageSize"

/**
 * The job list's page size, remembered across sessions in
 * `kv_settings`. The URL (`?size=`) wins when present; this is the default
 * a fresh visit to a queue starts from.
 */
export function useJobPageSize() {
  const queryClient = useQueryClient()
  const { data } = useQuery({
    queryKey: ["kv", KEY],
    queryFn: async (): Promise<PageSize> => {
      const r = await commands.getKvSetting(KEY)
      if (r.status === "error") throw new Error(r.error.message)
      const parsed = Number(r.data)
      return isPageSize(parsed) ? parsed : DEFAULT_PAGE_SIZE
    },
    staleTime: Number.POSITIVE_INFINITY,
  })

  const mutation = useMutation({
    mutationFn: async (size: PageSize) => {
      const r = await commands.setKvSetting(KEY, String(size))
      if (r.status === "error") throw new Error(r.error.message)
    },
    onSuccess: () => announceSettingsChange({ topic: "kv" }),
  })
  const mutate = mutation.mutate
  const setPageSize = useCallback(
    (size: PageSize) => {
      queryClient.setQueryData(["kv", KEY], size)
      mutate(size)
    },
    [queryClient, mutate],
  )

  return { pageSize: data ?? DEFAULT_PAGE_SIZE, setPageSize }
}
