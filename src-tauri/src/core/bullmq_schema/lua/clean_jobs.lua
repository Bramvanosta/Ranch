--[[
  Ranch — clean: remove finished jobs older than a cutoff, one batch per
  call (BullMQ `cleanJobsInSet` for completed / failed).

  KEYS[1] completed | failed   KEYS[2] events
  ARGV[1] job key prefix       ARGV[3] offset — jobs skipped so far
  ARGV[2] cutoff (ms, by finish time)   ARGV[4] batch size

  Members of a flow are left in place and counted as skipped.

  Returns { removed, skipped, scanned }.
]]
local prefix = ARGV[1]
local jobs = rcall("ZRANGEBYSCORE", KEYS[1], "-inf", ARGV[2],
                   "LIMIT", tonumber(ARGV[3]), tonumber(ARGV[4]))
local removed = {}
local skipped = 0
for _, jobId in ipairs(jobs) do
  if isFlowMember(prefix .. jobId) then
    skipped = skipped + 1
  else
    removeJob(prefix, jobId)
    removed[#removed + 1] = jobId
  end
end
if #removed > 0 then
  rcall("ZREM", KEYS[1], unpack(removed))
  rcall("XADD", KEYS[2], "*", "event", "cleaned", "count", #removed)
end
return { #removed, skipped, #jobs }
