import { Segmented } from "@/components/data/Segmented"
import { Button } from "@/components/ui/button"
import { useAttentionMutes } from "@/features/attention/useAttention"
import { useJobPageSize } from "@/features/jobs/useJobPageSize"
import { PAGE_SIZES, type PageSize } from "@/features/jobs/useJobs"

import { SettingRow, SettingsPane } from "./SettingsPane"

/** Settings › General: how lists open, what is muted. */
export function GeneralSection() {
  const { pageSize, setPageSize } = useJobPageSize()
  const { mutes, unmuteAll } = useAttentionMutes()
  const muted = Object.keys(mutes).length
  return (
    <SettingsPane title="General" testId="settings-general">
      <div className="flex flex-col">
        <SettingRow
          label="Jobs per page"
          hint="Where a job list starts. A list's own page size wins while it is open."
          testId="settings-page-size"
          control={
            <Segmented
              label="Jobs per page"
              value={String(pageSize)}
              onChange={(v) => setPageSize(Number(v) as PageSize)}
              options={PAGE_SIZES.map((n) => ({ value: String(n), label: String(n) }))}
            />
          }
        />
        <SettingRow
          label="Muted in Needs attention"
          hint={
            muted === 0
              ? "Nothing is muted."
              : muted === 1
                ? "1 item is hidden until its mute ends."
                : `${muted} items are hidden until their mutes end.`
          }
          testId="settings-mutes"
          control={
            <Button type="button" variant="default" disabled={muted === 0} onClick={unmuteAll}>
              Unmute all
            </Button>
          }
        />
      </div>
    </SettingsPane>
  )
}
