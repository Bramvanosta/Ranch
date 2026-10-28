# Changelog

All notable changes to Ranch are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

<!-- Releasing: add `## [vX.Y.Z] - date` above the version's sections and write ONE short paragraph right
     under it, before the first `###`. That paragraph is what the update dialog shows (scripts/publish-update.mjs). -->

## [v0.4.0] - 2026-10-28

Fast on a far Redis, and clearer everywhere. In short: **one connection per Redis, kept open**, so a remote server answers like a local one and slow no longer means "unreachable"; **a new home** with every connection and what needs attention across them; **a connection overview** with live events, schedulers, workers, flows, a timeline and a map; **failed jobs grouped by cause** and running jobs with their progress; native windows, ⌘K to reach everything, and destructive actions that confirm in a sheet; **the Agents page**, where you choose what job data agents see and read the notes they leave on failures, and **Ask AI**; tighter safety for agents; and an install through **Homebrew** on macOS. Some keyboard shortcuts and MCP tool outputs change: the changelog lists them.

### Added

- **Failed jobs grouped by cause.** On a queue's Failed tab, *By cause* (or View › Group by Cause, ⌥⌘G) folds the failures whose reasons only differ by their values (ids, numbers, quoted values, addresses, URLs, timestamps) into one group: how many, the job types, when it last happened and the last 24 hours as small bars. Open a group to see its jobs; *Retry N* and *Remove N* act on the whole group after a confirmation.
- **Running jobs say how far they are.** The Active tab shows each job's progress (a bar when the worker reports a percentage, even inside an object like `{percentage, message}`, with its message underneath), how long it has been running against the usual time of its type (the median over the metrics window, amber when slower), the worker that took it and the attempt. A job whose progress has not moved for 3 minutes while its lock is still renewed turns amber with "no progress for 3 min". Its inspector opens on Progress: the progress data, the lock, and the logs as they come.
- **Add Job is a sheet made to be sure** (Queue › Add Job, ⌘J). The name offers the names the queue already uses; the payload editor numbers its lines and says as you type "Valid JSON · 4 keys · 98 bytes", or what is wrong and on which line; the button and ⌘↩ wait for valid JSON. New options: backoff (fixed or exponential), a custom job ID (refused when already taken), keep when done / failed. "Same thing in code" shows the matching `queue.add(...)`, to copy.
- **AI features are a choice** (Settings › AI features). One switch turns them off: the Agents menu and sidebar item, the ✦ marks, agent approvals and agent filters leave every window at once, and Ranch's MCP server refuses every call from an agent ("AI features are turned off in Ranch"): requests that were waiting are denied, and your agent clients' configurations are left as they are, so turning AI back on needs nothing else. A new install asks "Use Ranch with AI agents?" after the first connection, Yes or No.
- **The Agents page** (sidebar › Agents). Everything about agents in one place, out of Settings: **Approvals**: what is waiting for you, in full (what it would do, the jobs it touches, the exact call, the time left, a note when the connection is protected), with *Deny* and *Approve once*, then the earlier decisions and how they ended; **Activity**: the agents' calls, All / Writes / Errors; **Notes**; **Setup & safety**: the agent clients and the safety rules, with each connection's access in one word (Read-only, Ask, Free). The approval sheet shows the same details.
- **You choose what job data agents see** (Settings › AI features › Job data shared with agents). *Mask fields whose name matches* is the default: the values of fields such as email, phone, token, password, secret, IBAN or card numbers reach an agent as •••, at any depth, and you edit the list. *Names, states and errors only* sends no payload, return value, progress or log at all; *Everything* sends them as they are. Failure reasons and stack traces are always shared.
- **Agents leave notes on failures.** An agent that worked out why jobs fail can attach its diagnosis to the cause (new MCP tools `list_failure_causes` and `annotate_cause`): what is wrong and the steps to fix it. The note shows under the cause on the Failed tab (by cause) and in the overview's Needs attention, with who wrote it and when, and *Retry N*, *Ask a follow-up*, *Copy* and *Dismiss*; Agents › Notes lists them all. Notes need no approval (they change nothing in Redis), but a connection read-only for agents refuses them.
- **Ask AI.** On a job, on a failure cause and on a connection, *Ask AI* hands the problem to your agent with the context ready: what it is, the last error and stack, the last logs, the queue's state and a link back into Ranch, with job data masked as in Settings › AI features: **Claude Code** starts in Terminal, in the group's code folder; **Claude Desktop** and **Cursor** get the prompt on the clipboard and open; or just *Copy prompt*. Agents › Ask AI About Selection (⇧⌥⌘A) uses your default agent. Set the default and each group's code folder in Agents › Setup & safety.
- **Homebrew.** `brew install --cask bramvanosta/tap/ranch` installs the macOS app and follows stable releases; `ranch@beta` follows release candidates too, and stable releases as well, so it is never behind. Each release updates the casks.
- **Ranch server on arm64.** The `ghcr.io/bramvanosta/ranch` image is published for amd64 and arm64.

### Changed (speed on a remote Redis)

- **One connection per Redis, kept open.** Ranch used to open a new Redis connection for every refresh, and one more per queue for the counters, paying the TCP, TLS and SSH handshakes each time. A connection now keeps one, shared by everything and reopened by itself after a drop: ten refreshes that opened 55 connections open one.
- **A far Redis is slow, not "unreachable".** Replies may take up to 20 s and opening a connection up to 10 s (it was 0.5 s and 1 s), so a link with a long round trip works instead of failing. The latency Ranch shows is the round trip of a PING, without the handshake.
- **The queue list costs one round trip.** Ranch remembers the queues it found and looks for new ones in the background every 30 seconds, instead of walking the whole Redis on every refresh (100 keys per round trip; a large Redis took seconds to minutes). Every queue's counters are then read together: on a 600 ms link the list went from 3.6 s to one round trip. A queue deleted meanwhile leaves the list instead of showing zeros, and a counter that cannot be read is reported as an error instead of zeros.
- **A click never waits behind a refresh.** A connection used to answer one request at a time, so opening a job or a ping queued behind whatever was refreshing. Reads now run side by side (up to 16 per connection), and one whose screen has gone away is dropped instead of being run anyway. Actions stay in the order you made them.
- **Views read everything at once.** A job page reads its count, its jobs and their locks in two round trips (was three or four), a job and its lock in one; the failed jobs, the recent jobs, the Map and the Timeline read every queue at the same time instead of one after the other; the Workers tab's per-queue calls share one read of the Redis clients.
- **You can see how fresh the data is.** Next to Refresh, on every connection and queue, and on the connections page: **Live** while the counters keep up (with their age once it passes a couple of seconds), amber **Updated 12s ago** when refreshes run late, red **Last update 40s ago · retrying** when the last one failed; hover it for the reason.
- **A failed refresh keeps what you were looking at.** The queues, jobs, failed jobs, schedulers, recent jobs, causes and the overview dashboards stay on screen with a line saying they could not be refreshed, how old they are, and why; the error box is only for a view that has nothing to show yet. A refresh is retried once, not three times, so trouble shows sooner.
- **No more "0" while loading.** The connections page shows "-" until the counters are read, instead of "0 failed · 0 waiting". The overview's "refreshing…" no longer blinks at every refresh: it only shows while another time range loads.
- **The job list is live, and never shows another queue's jobs.** The page on screen refreshes by itself (at your refresh interval, at least every 5 s; not while searching) and as soon as its state's counter moves. Changing page, state or sort keeps the previous rows dimmed under a thin progress bar, with the page number of what is shown; opening another queue no longer leaves the previous queue's jobs on screen while it loads. A retried, removed or promoted job leaves the list at once, and comes back if the action fails; the action stays busy until the refreshed list is back.
- **One refresh per thing on screen.** Several parts of a screen showing the same data used to refresh it each on its own clock: the queue list of an open connection was read once or twice a second. Now each piece of data has one refresher: the next read starts when the previous one is done, as often as the most demanding view asks (a connection only in the sidebar every 10 s, the one you are looking at every second), and never more often than three times what the last read took, so a slow link is not flooded. Refreshing pauses while the window is hidden and catches up the moment it is back. The connections page and the sidebar now share one read per connection. The Map, Timeline, flows, failure causes and a job's details follow Settings › Refresh like the rest; an open job that is not running is refreshed too (every 10 s), so a waiting job that starts shows it.
- **The connection overview's metrics read every queue at once** (2 s → 0.4 s on a 100 ms link), and threshold notifications reuse the counters already on screen instead of reading every connection in turn.
- **Actions answer at once.** Pause and Resume flip the queue straight away and the button stays busy until the refreshed list confirms it (it used to say "Queue paused" while still showing Pause); a failure flips it back with the reason. While an action runs, the data it changes is not refreshed underneath it, so nothing flickers back. A job opens in the inspector instantly with what its row already shows, while the fresh read runs. Refresh no longer cancels and resends reads already on their way. The queue toolbar (Pause, Maintenance, Add job) is there from the first frame instead of popping in once the queue list arrives.
- **Live, without asking all the time.** While a connection is on screen (and the window is shown), the desktop app listens to the event streams BullMQ already writes for its own `QueueEvents` (no Redis setting needed) over a connection of its own. When a job is added, completes, fails or moves, the counters, job lists and open jobs of that queue are read again right away (in batches of at most one every 0.3 s on a busy Redis); polling of a followed connection drops to a safety net every 15 s. The server's web app keeps polling.
- **SSH tunnels stay up.** A keepalive every 30 s keeps an idle tunnel open and notices a dead bastion; small commands are no longer held back by Nagle's algorithm on the SSH and loopback sockets.

### Changed (home)

- **All connections as a table.** The home page opens on the fleet in four figures (connections and how many are offline, failed with the last hour, waiting, workers) above one table per group: connection with its environment and host, health (latency and its trend), queues, waiting, failed, workers, protection. An unreachable connection says why in plain words ("Connection refused. Is Redis running on port 6380?"), when Ranch tries again, and offers *Retry now* and *Edit*. An empty group offers *Add a connection here*. The cards are gone.
- **Needs attention, across connections.** The sidebar's *Needs attention* opens one page for everything that needs you: **Failing** (the failure causes of every connection), **Stuck** (a queue paused for more than 15 minutes, or with jobs waiting and no worker) and **Offline**, most urgent first (production before staging), each with where it is, since when, its trend and what to do: Open, Retry N, Resume, Retry now, Edit. Filters: All / Production / Staging, Hide Local. *Mute* hides an item for an hour, four hours, a day or a week ("Muted (n)" keeps them reachable), and the sidebar count leaves muted items out.

### Changed (connection overview)

- **A connection reads in five seconds.** Its Overview opens on five figures, Waiting, Active, Delayed, Failed (with how many failed in the last hour) and Completed per hour, each with its trend, then **Needs attention** (the failure causes, paused queues, queues with waiting jobs and no worker, each with *Open*) beside the **busiest queues** of the last 6 hours as heat strips. A queue that records no BullMQ metrics says "metrics off" instead of showing zeros. The info strip shows the Redis version, latency, memory, operations per second, workers and what agents may do.
- **Live events.** The overview shows what happens on the connection's queues as it happens (time, state, job and `queue/id`, the failure under a failed job), with agent calls marked ✦ and a pending agent approval on top (Deny, or Review ⌥⌘A). *Pause* freezes the list. Ranch reads the queues' event streams only while the overview is on screen and the window visible; when more happens than it reads, it keeps the newest and says so.
- **Queues as a table.** A connection's Queues tab is now a table to compare queues: status (Paused, No worker, Working, Idle), waiting, active, delayed, failed, done per hour and failure rate over the last hour, the last 6 hours as heat strips on one axis, and workers. Every number sorts; a zero is dim. The cards are gone.
- **Schedulers of a connection.** A new Schedulers tab shows what will run in the next 24 hours (one lane per scheduler, a dot per run, a paused one hatched with who paused it) and a table: scheduler and the job it makes, queue, schedule with its pattern in plain language ("weekdays at 08:30"), next run, last run and how it ended, state. A scheduler opens beside the list with its next five runs, its job template, its last runs, and Edit, Pause / Resume, *Run once now* and Remove.
- **Workers of a connection.** A new Workers tab shows how many workers are connected and working, the queues without a worker and the client libraries in use (an older version is highlighted), then the workers grouped by queue (name, address, idle time, Working / Idle / Paused, library). A queue with jobs waiting and nobody to take them gets a banner with when a worker was last seen. A worker opens beside the list with the jobs it processed lately and its raw `CLIENT LIST` fields.
- **Flows of a connection, and blocked parents.** A new Flows tab lists the connection's recent flows with how their children are doing; a parent stuck behind a child that failed for good, with no `failParentOnFailure` or `ignoreDependencyOnFailure` to let it go on, is marked *Blocked* and listed first. A flow opens as its tree (each job's state and duration, what finished children returned, the failed child's error) with a banner that explains the block and offers *Retry child* and *Remove child*.
- **Timeline.** A connection's new Timeline tab shows every queue on one chart (completed under failed, over 1 h, 6 h, 24 h or 7 days), with what people did (pauses, retries, removals, unlocks) and, with AI features on, what agents did below it, and the queues underneath on one shared scale, the failing ones first. When the tab opens, the moment with the most failures is already selected and summarised beside the chart: failed, done and wait against the rest of the range, the causes with a link to their jobs, the queues involved and what happened around it. Drag across the chart to look at another moment. Queues whose workers do not record BullMQ metrics are named, with how to turn them on.
- **Map.** A connection's Map tab shows its queues on the left and the worker pools that consume them on the right: each edge as thick as the queue's throughput and coloured by how much of it fails, dashed when the queue is paused or idle, flow links between queues, and a *No worker* node for queues nobody takes jobs from. Click a queue for jobs in and out per minute, how much fails and why, how long the oldest job has waited, and *Open queue* / *Show workers*. *Only queues with activity* keeps big connections readable; View › Fit Map (⌘0) shows the whole picture.

### Changed (queues)

- **A queue's address uses its id**: `/connections/<connection>/queues/<queue id>` (`bull%3Aemails`). An address or a `ranch://` link that names the queue (`queues/emails`) still opens it, and moves to the id. *Copy Queue Key* is *Copy Queue ID* (the same text on BullMQ).
- **The audit log names a queue by its id too.** Rows written before keep their queue's name and have no id; they are never rewritten. Cause notes are converted to the queue's id when Ranch starts.

### Changed (job inspector)

- **A job reads top to bottom.** Its header has the name, the id, the state, the last error and Retry, Remove, *Copy JSON*. The tabs are **Trace** (the job's life on one time axis: the wait, each attempt, each backoff hatched, and "added → failed in … · … of actual work"; the default for a failed job) · **Data** (payload and result) · **Error** · **Logs** · **Raw** (the whole job as JSON or YAML, copied as shown). BullMQ keeps the times of the last attempt only, so earlier attempts are placed from the backoff setting and marked ≈. Old `?tab=` links still open the matching tab.

### Changed (connections)

- **The environment is a tag, not a level.** A group's connections are listed right under it, each with its environment as a coloured tag, ordered by environment then name: a project in Local, Staging and Production is one level deep instead of three, and an environment can hold several connections. A connection's menu has **Environment** (retag it, rename the tag, or create one), **Move to Group** (it keeps its tag; the target group gets it if it did not have it) and **Environment Color**. Dragging a connection onto another group moves it the same way. The connections page shows the same tags.
- **A new connection in two steps** (File › New Connection, ⇧⌘N). Step 1 is the server: paste a `redis://` or `rediss://` URL or fill the fields, with TLS and the SSH tunnel folded away until you need them; Enter tests. Step 2 shows what the test found (latency, Redis version, TLS, the BullMQ queues with a few of their names, schedulers and workers), then the name, the group and environment, "Read-only until I unlock it" (on by default for Production) and agent access. A failed test says what to check in plain words, and Back keeps what you typed. Editing a connection uses the same two steps; a blank password now tests with the saved one.
- **A connection is stored as its backend and where it is reached.** When Ranch starts, each saved connection's Redis settings (host, port, database, user, TLS, custom CA, SSH tunnel) move into one endpoint for its backend, BullMQ on Redis. Nothing to do: passwords stay in the keychain, and the lock, the agent setting and its protection are kept as they are. An earlier version of Ranch cannot open the database afterwards.
- **Saving a connection saves what the form shows.** Clearing the custom CA of a saved connection, or turning TLS off, removes the CA instead of keeping the saved one.

### Changed (across the app)

- **The audit log as a table.** Time, who (you, or the agent ✦), action, target, where, result, filtered by text, by who (Everyone / You / Agents), by connection and by action, and exported as CSV or JSON. An entry opens beside the table with when, the connection, the result, the approval behind an agent write (who asked, who approved, after how long) and the exact arguments. Decided approvals are now kept 90 days for this (undecided requests still go after an hour).
- **⌘K reaches everything.** The palette groups its results as **Go to** (queues across connections with their failure counts, connections, groups, pages), **Actions** (what applies to what is on screen, with the menu's shortcuts) and **Jobs**: type `#1211` to find that job in the queues of the connection on screen. ⇥ on a result lists what can be done with it (open in a new tab, open its failed jobs by cause, copy its key, retry a job…). Long names wrap instead of being cut.
- **Settings in a window of their own** (Ranch › Settings…, ⌘,). One Settings window (⌘, again brings it to the front) with toolbar tabs: **General** (jobs per page, unmute everything in Needs attention), **Appearance**, **Refresh**, **Alerts**, **Updates** (your version, *Check now*, install a new one after a confirmation), **Advanced** (show dismissed hints and the welcome screen again, clear ⌘K's recent items) and **Agents**. A change applies to every open window at once: theme, refresh interval, alert thresholds, what agents may do.
- **One look everywhere.** The last old-style pieces are gone: panels instead of cards on the queue overview, the same underlined tabs in the job inspector and agent activity as on every screen, and sheets instead of centred dialogs for deleting a group, confirming an update, editing a scheduler, a worker's details, an agent's approval request and the keyboard shortcuts. Long names in these places wrap instead of being cut.
- **Readable in light and dark.** Every screen and sheet now reaches the WCAG AA contrast for text in both themes, checked automatically: some error texts were white on light backgrounds, a few labels and keyboard hints were faded below it, and state labels (running, waiting for children, completed) are darker on their tinted pills in light.

### Changed (window)

- **Native windows and tabs.** File › New Window (⌘N) and New Tab (⌘T) show the current screen again; Queue › Open in New Tab and Job › Open in New Window (⌥↩), also in the right-click menus. On macOS tabs are the system's (its tab bar, ⌘W, Merge All Windows); elsewhere a tab is a window. The in-app tab strip is gone. Only the main window is remembered for the next launch.
- **The production lock is a toolbar button**, not a red banner: `Read-only ⇧⌘U` while locked, `Unlocked · 13 min` once unlocked (a click locks again, so does ⇧⌘L); the sidebar row shows a lock while it is locked. Same rules as before: unlocked until Ranch quits, every unlock and lock audited.
- **No status bar.** The version is in Ranch › About; an available update shows at the bottom of the sidebar and in Ranch › Check for Updates; pending agent approvals are the count on **Agents** and Agents › Pending Approvals (⌥⌘A).
- **Destructive actions confirm in a sheet** at the top of the window: Drain, Clean, Retry all failed, removing jobs and removing a connection say what goes and what stays, with counts, and the button says how much (*Drain 400 jobs*). Cancel has the focus when something is deleted, so Enter no longer removes the selection. **On a locked production connection** these actions no longer need an unlock first: type the queue's name in the sheet and Ranch unlocks for that action only, then locks again; both are in the audit log.
- **A new logo.** Three bars, the head of the queue sliding out of the line: in the app icon, the favicon, the welcome screens and the sidebar header of Ranch web and Ranch server, with the name set as `ranch` in JetBrains Mono. The small sizes (16 and 32 px) are redrawn on the pixel grid to stay sharp. The browser tab is titled just *Ranch*.

### Changed (keyboard shortcuts, breaking)

- **Retry a job is ⇧⌘R**, no longer ⌘R: ⌘R now **refreshes** the screen, as everywhere on macOS.
- **Promote a job is ⌥⌘P**, no longer ⌘P (Print).
- **⌘F puts the cursor in the job filter.** ⇧⌘S (Save As) no longer switches the search between this queue and all queues: click the scope instead.
- Every command now lives in one registry, shown in the **menu bar** (desktop) or the **Ranch** menu at the top of the sidebar (server image), in the palette, and in **Help › Keyboard Shortcuts**, always with the same chord. New: ⌘1–⌘5 for the job states, ⌘J add a job, ⌥⌘R retry all failed, ⌥⌘K copy the queue key, ⇧⌘C copy the job ID, ⌥⌘C copy the payload, ⌘E edit the connection, ⌃⌘S hide the sidebar, ⌥⌘L audit log, ⇧⌘N new connection.

### Changed (MCP tools, breaking)

- **A job state is a code** (`list_jobs`'s `state` argument, and `state` in every job, flow and page an MCP tool returns). The codes are the same strings as before: the common lifecycle `waiting`, `active`, `completed`, `failed`, `delayed`, `paused`, and BullMQ's own `prioritized`, `waitingChildren`, `stalled`. `list_jobs` lists them as an `enum` in its schema; any other value is refused as before.
- **A queue is named by its id** (every MCP tool). `list_queues` gives each queue an `id` (`bull:emails` on BullMQ: `{prefix}:{name}`, the base of its Redis keys) and a `name`; every tool that took `queueRef: { prefix, name }` takes `queueId` instead, a string to pass as given. Job rows (`list_jobs`, `list_failed_jobs`) carry `queueId` and `queue` instead of `queueRef`; `get_flow`'s nodes and `list_failure_causes`' jobs carry `queueId`; `annotate_cause` takes the `queueId` of one of the cause's jobs (and, optionally, the `queue` name to show); approval previews carry `queueId` next to the queue's name.
- **A connection says its backend and where it points** (`list_connections`). Each connection carries `backend` (`bullmq-redis`) and `endpoint`, one line without credentials (`redis://redis.internal:6379/0`, `rediss://` with TLS, `… via SSH` through a tunnel), instead of `host`, `port`, `db`, `username`, `tls`, `tlsCustomCa` and `sshTunnel`: an agent no longer receives the user name, the CA, or the SSH bastion, user and key path. The rest of the row is the same, without `sidebarCollapsed`.
- **A queue's jobs per state are a map.** `list_queues` returns `counts` (`{"waiting": 12, "active": 1, "failed": 3, …}`, keyed by state code) instead of `counters` with one field per BullMQ state. A state that is absent was not counted (BullMQ's `waitingChildren`): list its jobs to know.

### Fixed (queues)

- **Resuming a queue resumes it for BullMQ too.** Ranch marked a resumed queue with `paused = 0`, but BullMQ treats a queue as paused as long as that field exists, whatever it holds: its workers kept the queue paused and new jobs went to the paused list. Resume now removes the field, as BullMQ does, and also clears a `paused = 0` left by an earlier Ranch.
- **A connection's Schedulers view and a queue's Schedulers tab open together no longer blank each other.** They shared a cache entry in two different shapes.
- **Live refresh no longer misses the first changes.** When Ranch started following a connection, what happened before its first read of a queue's events, or in the very millisecond it started, was skipped, so a screen could wait for the next poll.
- **The Workers tab reads the server's client list once**, however many queues the connection has.

### Fixed (layout)

- **The desktop app shows its name.** The sidebar header carries the Ranch mark and name, beside the traffic lights on macOS (the menus live in the menu bar, so it is not a button there).
- **One scroll per screen.** A connection's overview could scroll as a whole window *and* inside its view *and* inside Live events: screen-reader labels placed outside the scrolling view stretched the window, and Live events had its own scroll box. Views now contain what they position, the window never scrolls, and Live events shows its 12 newest lines.
- **Every page uses the full width.** The connections page and Needs attention were centred in a narrower column (and Agents capped), unlike every other screen.
- **The Map names worker pools usefully.** Workers without a name were grouped by host. In containers every client comes from the same gateway address, so they all became one "Unnamed workers on 192.168.…" pool. They are now grouped by the queue they serve ("cron workers"), and the pool's details say how to name them.

### Security (agents)

- **Ranch's MCP server only opens connections the app saved.** A connection's seal covers where it points (its backend and endpoint), and `ranch-mcp` refuses a connection whose seal does not hold, reads included, with `connectionUnverified`: a process that edits Ranch's database can no longer have a saved password sent to its own host, or point a connection open to agent writes at another server. A connection's password is the keychain entry named after its id, never one the database names.
- **A lock set in the app stops agent writes at once.** With approvals off, an agent's write is refused on a connection the app locked, even when the agent had used the connection before the lock.
- The first launch after the update carries over the seals that still hold. A connection whose seal does not is refused to agents until you save it again in Ranch.
- **An approval covers what you were shown.** Its seal also covers how the request was shown (connection, group, environment, color, preview): a request whose approval sheet another process rewrote runs nothing. A request still waiting when Ranch updates is refused once approved; the agent asks again.
- **The key behind these seals has its own keychain entry**, apart from connection passwords, copied there on first launch; the old entry is kept for one more release, so a `ranch-mcp` you already allowed keeps working while macOS asks about the new one. If the key goes missing, Ranch resets agent safety to its strictest settings (AI off, every connection locked and read-only for agents) and says so, instead of trusting the database as it finds it: save each connection again in Ranch to let agents use it.
- **When its key cannot be read, `ranch-mcp` treats AI features as off** and shares job names only.
- **With approval mode "Does anything on a connection", `annotate_cause` waits for approval too.** Listing connections and environments stays free.
- **Every write refused by a connection's lock is in the audit log** as a failure, single or bulk, from the app or from an agent.
- **`ranch-mcp` no longer updates the database.** When the database does not match its version, it says to open Ranch or to update it, then to restart the agent client, and stops instead of failing call after call.

### Security (connections and Ranch server)

- **A saved Redis password is only reused over the same route**: same host, port, TLS and CA, through the same SSH bastion and jump host. Changing one asks for the password again.
- **Ranch server refuses HTTP Basic sign-in for a minute to an address that sent 10 wrong passwords**, and to everyone past 200 from all addresses together. Behind a reverse proxy, clients share the proxy's address.
- **Signing out of Ranch server revokes the session on the server**: a copy of its cookie no longer opens anything. Existing sessions sign in again once.
- **The database is readable by its owner only** (`ranch.db` and its `-wal` and `-shm` files).

### Fixed (data)

- **Ranch keeps a copy of its database before it updates it** (`ranch.db.bak-schema-<n>`, beside it).
- **When the database cannot be opened, Ranch says why and where it is**, and offers to show it, set it aside and start fresh, or quit, instead of closing at once. A database written by a newer Ranch is refused with a clear message.
- **Ranch writes a log file** (`~/Library/Logs/dev.ranch.app/ranch.log` on macOS), crashes included.
- **Concurrent writes from the app and agents wait their turn** instead of failing with "database is locked", approvals included.
- **Saving a connection no longer holds the database while the keychain asks for permission.**
- **Settings saved while the keychain was locked are protected once it opens**, if nothing else changed them meanwhile; saving never waits on the keychain.
- **Duplicating a connection keeps its pinned SSH host keys**; removing one also forgets the jump host's pin.
- **The audit log shows the right environment color** after a connection is moved or its environment recolored, and one unreadable row no longer hides the page.

### Fixed (across the app)

- **Retry N and Remove N on a failure cause act only on the open queue's jobs**, never on a queue of the same name under another prefix.
- **Agents' notes show on their cause in the queue view**, matched by queue id.
- **Retry in the command palette is offered for failed jobs only**, and says how it went.
- **The Waiting children tab is always reachable**: a state the backend does not count keeps its tab, without a number.
- **A paused queue counts the jobs its pause holds**, on the overview as in Needs attention.
- **A cron scheduler saved by Ranch always carries its timezone**, UTC when none is given, so the first run Ranch computes and the runs workers compute agree; a pattern saved elsewhere without one says it runs in the worker's local time.
- **Keyboard hints show Ctrl outside macOS.**
- **Menus leave out what the connection's backend does not support**, and Clean is red like Drain.
- **Ask AI prompts give the `connectionId` and `queueId`** the Ranch MCP tools take; an agent's queue name is quoted as data in a note's follow-up.
- **Texts point to the right places** (Agents › Setup & safety, Settings › Alerts) and name states by their label.

## [v0.2.0] - 2026-09-21

The first real release. In short: connection **groups and environments**; **agents (MCP) whose writes wait for your approval**, with a full activity log; **the whole BullMQ toolbox**: add jobs, drain / clean / retry all, create and edit schedulers, workers, flows, metrics over any range, search across a whole state, one view of everything that failed, a real home page; **Ranch server**, the same app as a Docker image with OIDC sign-in; and releases built in CI (signed and notarized macOS app, Linux bundles) for an app that now **updates itself**.

### Added

- **Connections to any Redis.** Paste a `redis://` or `rediss://` URL or fill the fields, and test before saving: Ranch shows the latency and the Redis version, or says what went wrong. Passwords are kept in the OS keychain.
- **TLS, with an optional custom CA.** Leave the CA empty to use the system's trust roots (most managed Redis), or paste your own certificate chain.
- **SSH tunnels**, with a key file or a password and an optional jump host. Ranch pins the bastion's host key on the first connect and refuses a key that changed.
- **Groups and environments.** The sidebar shows group › environment › connection. Create, rename, collapse and delete groups and environments, pick them in the connection form, and move a connection by dragging it or with *Move to…*. The colour belongs to the environment.
- **Manage connections**: edit, duplicate, remove and reorder them. An edit applies at once, without restarting Ranch.
- **Production is read-only until you unlock it.** A connection in a Production environment starts locked: writes are refused until you unlock it (⇧⌘U), and it stays unlocked until Ranch quits. Every unlock and lock goes to the audit log.
- **A home page.** Ranch opens on every connection as a card, grouped, with live health (queues, waiting, active, failed) and search. With no connection yet, it is the welcome screen.
- **Queues with live counters.** A connection's queues show their waiting, active, delayed, failed and completed jobs, with paused queues and queues that no worker consumes flagged. Ranch reads BullMQ 5 and 6.
- **Job lists for every state**, including prioritized and paused. Search by id, name or failure reason across the whole state (its newest 10,000 jobs), sort by name, queued time or duration, choose the page size; all of it lives in the URL. Move with `j` / `k` and open with Enter.
- **A job page that reads top to bottom**: name, state and id, why it failed and how many attempts are left, a timeline of what happened and when, its details, one stack trace per failed attempt, then the payload, result, logs, options and flow.
- **Retry, remove and promote jobs**, one at a time or in bulk: select with the checkboxes, shift-click or ⌘A, even across queues, see what will happen, confirm, and follow the progress. Removing always asks first.
- **Add a job** to a queue: name, JSON payload checked as you type, and optionally a delay, attempts, a priority or LIFO. Ranch writes it exactly as BullMQ's `Queue.add` does.
- **Queue maintenance**: retry all failed jobs, clean completed or failed jobs (all, or older than an hour, a day, 7 or 30 days), and drain what has not started. You see the count before anything happens. Active jobs, the pending run of a scheduler and jobs in a flow are left in place.
- **Pause and resume queues.**
- **Schedulers**: list, create, edit, pause, resume and delete them, on a cron pattern (with a timezone) or a fixed interval, with a JSON data template. The next five runs show as you type.
- **Workers**: the BullMQ workers connected to each queue (from `CLIENT LIST`), and a "no worker" warning when nothing consumes a queue. Managed Redis that forbid `CLIENT LIST` get an explanation instead of "0 workers".
- **Flows**: the whole tree of a job that is part of a BullMQ flow, across queues, and the flows that start on a queue with how their children are doing.
- **Everything that failed, in one view.** A Failed tab on each connection lists the failed jobs of all its queues, filtered by queue, job type or failure reason, with bulk retry and remove.
- **Metrics over any range**, from 5 minutes to 7 days, on queue and connection overviews: throughput, failure rate, processing time and queue delay (average, min, max), the slowest jobs and the most failing job types. Counts are exact when your workers record BullMQ metrics, and estimated (and marked so) otherwise.
- **Jobs per state and recent jobs** at the top of each overview: what is running now and the latest jobs to finish, with how long each ran.
- **Copy a job's id, payload or stack trace** from the job page or a row's right-click menu.
- **A built-in MCP server** (`ranch-mcp`) for Claude Desktop, Claude Code, Cursor, Codex and Windsurf, installed in one click or by hand. Agents can read connections, queues, jobs, failures, logs, schedulers, workers, flows and metrics, and ask to retry, remove or promote jobs in bulk, pause and resume, run maintenance, edit schedulers and add jobs. See `docs/mcp-tools.md`.
- **Agent writes wait for your approval.** Ranch shows who asks, on which connection, with which tool and a preview of the target. Deny is the default and silence never approves; one approval allows one operation. Choose to be asked for writes (the default), for everything that touches a connection, or never.
- **Connections read-only for agents.** A switch per connection, plus a global default; connections in Production start read-only for agents.
- **An agent activity log**: every call an agent made, reads included, grouped by session, with filters and JSON / CSV export.
- **An audit log** of every write, by you or by an agent (with the client's name and the approval behind it): filters, CSV / JSON export, append-only.
- **Command palette** (⌘K) for connections, groups, queues and actions.
- **Tabs** for the queues you opened.
- **`ranch://` links** to a connection, a queue or a job, by id or by group and environment names.
- **Notifications** when a counter crosses a threshold you set.
- **A first-run wizard**: your first connection, then your agent clients.
- **Settings** for the theme (system, light, dark), how often Ranch refreshes (or not at all, with a Refresh button on every page), notifications and agents.
- **The desktop app updates itself.** It checks at start, at most once a day (`RANCH_DISABLE_UPDATE_CHECK=1` or `--no-update-check` turn it off), shows the release notes, and installs only after your click, once the download's signature is verified. macOS (universal) and the Linux AppImage; a `.deb` or `.rpm` install is updated by its package manager. There is a stable and a pre-release channel: a stable build is never offered a pre-release.
- **Ranch server**, the same app as a Docker image: `docker run -p 8080:8080 -v ranch-data:/data -e RANCH_PASSWORD=… ranch`. A password or sign-in with any OIDC provider (with optional allow-lists by e-mail and group), and Redis passwords encrypted when `RANCH_SECRET_KEY` is set. Agents and self-update are desktop-only.

### Changed

- **Agents' list calls cost far less.** `list_jobs` and `list_failed_jobs` return summaries (no stack traces, no options, a failure reason capped at 800 characters), and `inspect_job` returns the rest, with identical stack traces collapsed. Five failed jobs went from 35 KB to 5 KB.
- **Agent answers are easier to use.** Each row carries the `queueRef` and `jobId` the next call takes, job data comes back as JSON instead of strings of JSON, and paging arguments are optional.

### Fixed

- Jobs that had never run (waiting, delayed, prioritized) were missing from the lists of the pre-releases.
- Release builds of the pre-releases could crash at start or fail to sign on macOS.
- Retrying or promoting a job on a paused queue could lose it; it is now parked the way BullMQ does it.
- Pausing a scheduler hid it from Ranch and still ran its pending job once; resuming never made it fire again.
- A paused queue with nothing waiting was shown as running, and its counters left out paused and prioritized jobs.
- `cmd-Backspace` in a job list could delete a job without asking.

## [v0.1.0] - 2026-05-03 (Scaffold)

The project scaffold: Tauri v2, React and Rust, with no user-facing features yet.
