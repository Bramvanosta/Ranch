--[[
  Ranch — drain: remove the jobs that have not started, one batch of one
  container per call (BullMQ `drain`).

  KEYS[1] container (wait | paused | prioritized | delayed)
  KEYS[2] repeat (job schedulers)
  ARGV[1] job key prefix   ARGV[3] offset — jobs skipped so far, still in place
  ARGV[2] "list" | "zset"  ARGV[4] batch size

  Left in place and counted as skipped: the next run of a job scheduler
  (removing it would silently stop the schedule — as upstream) and members
  of a flow.

  Returns { removed, skipped, scanned }.
]]
local prefix = ARGV[1]
local isList = ARGV[2] == "list"
local offset = tonumber(ARGV[3])
local batch = tonumber(ARGV[4])

local scheduled = {}
local schedulers = rcall("ZRANGE", KEYS[2], 0, -1, "WITHSCORES")
for i = 1, #schedulers, 2 do
  scheduled["repeat:" .. schedulers[i] .. ":" .. schedulers[i + 1]] = true
end

local jobs
if isList then
  jobs = rcall("LRANGE", KEYS[1], offset, offset + batch - 1)
else
  jobs = rcall("ZRANGE", KEYS[1], offset, offset + batch - 1)
end

local removed = {}
local skipped = 0
for _, jobId in ipairs(jobs) do
  if scheduled[jobId] or isFlowMember(prefix .. jobId) then
    skipped = skipped + 1
  else
    removeJob(prefix, jobId)
    removed[#removed + 1] = jobId
  end
end

for _, jobId in ipairs(removed) do
  if isList then
    rcall("LREM", KEYS[1], 1, jobId)
  else
    rcall("ZREM", KEYS[1], jobId)
  end
end
return { #removed, skipped, #jobs }
