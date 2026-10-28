--[[
  Ranch: promote delayed jobs by id (BullMQ `promote`).

  KEYS[1] delayed  KEYS[2] wait    KEYS[3] paused  KEYS[4] meta
  KEYS[5] active   KEYS[6] events  KEYS[7] marker  KEYS[8] prioritized
  KEYS[9] pc (priority counter)
  ARGV[1] job key prefix   ARGV[2..] job ids

  A promoted job goes where a job added now would go: `wait` (or `paused`),
  or `prioritized` when it has a priority.

  Returns one code per id: 1 promoted, -1 no such job, -3 not delayed.
]]
local prefix = ARGV[1]
local maxEvents = getOrSetMaxEvents(KEYS[4])
local results = {}
for i = 2, #ARGV do
  local jobId = ARGV[i]
  local jobKey = prefix .. jobId
  local code
  if rcall("ZREM", KEYS[1], jobId) ~= 1 then
    code = rcall("EXISTS", jobKey) == 1 and -3 or -1
  elseif rcall("EXISTS", jobKey) ~= 1 then
    code = -1
  else
    local target, isPausedOrMaxed = getTargetQueueList(KEYS[4], KEYS[5], KEYS[2], KEYS[3])
    local priority = tonumber(rcall("HGET", jobKey, "priority")) or 0
    if priority == 0 then
      -- As upstream: the back of the line (workers pop from the other end).
      rcall("LPUSH", target, jobId)
    else
      local prioCounter = rcall("INCR", KEYS[9])
      rcall("ZADD", KEYS[8], priority * 0x100000000 + prioCounter % 0x100000000, jobId)
    end
    if not isPausedOrMaxed then
      rcall("ZADD", KEYS[7], 0, "0")
    end
    rcall("HSET", jobKey, "delay", 0)
    rcall("XADD", KEYS[6], "MAXLEN", "~", maxEvents, "*", "event", "waiting",
          "jobId", jobId, "prev", "delayed")
    code = 1
  end
  results[#results + 1] = code
end
return results
