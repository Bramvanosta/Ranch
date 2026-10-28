--[[
  Ranch: retry every failed job, one batch per call (moveJobsToWait).

  KEYS: as retry_jobs.lua
  ARGV[1] job key prefix
  ARGV[2] cutoff (ms): only jobs that failed before the operation started,
          so a job that fails again meanwhile is not retried in a loop
  ARGV[3] batch size

  Returns { retried, gone, remaining }; `gone` = ids whose hash no longer
  exists (dropped from the set); `remaining` = failed jobs still to do.
]]
local maxEvents = getOrSetMaxEvents(KEYS[4])
local jobs = rcall("ZRANGEBYSCORE", KEYS[1], 0, ARGV[2], "LIMIT", 0, tonumber(ARGV[3]))
local retried = 0
local gone = 0
for _, jobId in ipairs(jobs) do
  if retryFailedJob(KEYS, ARGV[1], jobId, maxEvents) == 1 then
    retried = retried + 1
  else
    gone = gone + 1
  end
end
return { retried, gone, rcall("ZCOUNT", KEYS[1], 0, ARGV[2]) }
