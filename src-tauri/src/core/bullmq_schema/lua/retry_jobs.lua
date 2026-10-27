--[[
  Ranch — retry failed jobs by id (single retry, bulk retry).

  KEYS[1] failed  KEYS[2] wait   KEYS[3] paused  KEYS[4] meta
  KEYS[5] active  KEYS[6] events KEYS[7] marker
  ARGV[1] job key prefix ("bull:emails:")   ARGV[2..] job ids

  Returns one code per id: 1 retried, -1 no such job, -3 not failed.
]]
local maxEvents = getOrSetMaxEvents(KEYS[4])
local results = {}
for i = 2, #ARGV do
  results[#results + 1] = retryFailedJob(KEYS, ARGV[1], ARGV[i], maxEvents)
end
return results
