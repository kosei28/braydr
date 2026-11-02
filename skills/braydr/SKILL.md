---
name: braydr
description: Operate one Braydr channel. Use at the start of a Braydr session and for any Braydr post, thread, or resolution work.
---

# Braydr

You are the only agent for one Braydr channel. Every `braydr_*` tool is scoped
to that channel, so you never pass a channel id.

## The model

- The main timeline is the channel's top-level conversation.
- A thread starts from one main-timeline message. The parent stays in the main
  timeline; replies live in the thread.
- A thread is unresolved until someone resolves it.
- Every post you make sets `agent_continues`. `true` means you will keep
  working on it after this post whether or not the user replies (an interim
  report, a "starting now"). `false` means this post ends your turn and you
  wait for the user.
- "Awaiting you" means the thread is unresolved and the last post is from the
  user (unanswered) or from you with `agent_continues=true` (continuing). That
  is your work queue.
- "Awaiting the user" means the thread is unresolved and the last post is from
  you with `agent_continues=false`. Do not resolve it while you are waiting for
  an answer.
- The main timeline awaits you the same way, unless its newest post has a
  child thread: `braydr_channel_get` reports it in `main_awaiting_agent` with
  `state` `unanswered` or `continuing`.
- Unread is a user-only concept. Do not reason about it.

## When the session starts

This session may be the first for this channel, or it may replace an earlier
one. Do not assume which: read the state.

1. Call `braydr_channel_get`.
2. If the main timeline is empty and no threads exist, do not post. Wait for the
   first `[braydr] new post` or `[braydr] new thread` notification.
3. Otherwise catch up:
   - Read the first page of `braydr_thread_list(filter="unresolved", limit=20)`.
     This is orientation; do not try to read every page.
   - Read the first page of `braydr_thread_list(filter="awaiting_agent",
     limit=20)`. These are the threads waiting on you.
   - Read the latest 50 main-timeline posts with
     `braydr_channel_list_posts(limit=50)`.
   - For each awaiting thread: read `braydr_thread_list_posts`, do the work, post
     one result, and resolve if finished. A continuing thread is work you
     promised and did not finish; pick it up from your last post. Every item must get a post; if you
     cannot finish, post the blocker or one focused question.
   - When the page is done, query `awaiting_agent` again from the first page. Do
     not advance a cursor while resolving threads: the order changes. Repeat
     until the queue is empty.
   - If `main_awaiting_agent` is set, handle the main timeline: for
     `unanswered`, reply in main or create a thread on that post; for
     `continuing`, finish the work you said you would continue.
   - Do not post a summary of what you read. Post only results and questions.

If a listed thread was resolved by the user while you were reading, re-read it
before posting. If an awaiting thread has no reply from you yet, do not resolve
it without posting.

## When a notification arrives

A `[braydr] new post` or `[braydr] new thread` notification contains the post
text and ids. It is not enough context to reply.

1. Read the scope:
   - main: `braydr_channel_list_posts` (the newest page is enough unless the
     post refers to older context)
   - thread: `braydr_thread_list_posts`
2. Decide where the reply belongs:
   - If the request is in a thread, reply in that thread.
   - If the request is in main, reply in main, or create a thread on that main
     message when the work deserves its own thread.
   - If that main message already has a thread, reply in the existing thread
     instead of creating a new one.
   - Do not answer a new main request only inside an unrelated thread.
3. Work.
4. Post one consolidated result with `agent_continues=false`. Pass `after` when
   the tool requires it.
5. If the request is in a thread and is complete, resolve the thread.

A `[braydr] nudge` lists work that is still awaiting you. Each entry has a
`state`: `unanswered` (the user's post) or `continuing` (your post with
`agent_continues=true`). Read each listed thread and the main timeline, then
handle them. Do not reply to the nudge
itself.

## Posting rules

- `after` must be the id of the newest message you actually read in that
  timeline. If the timeline is non-empty, `after` is required. On
  `stale_after`, re-read and retry with the new id.
- There is no retry key. On `stale_after`, re-read the timeline; if your post is
  there, do not post again.
- If `braydr_thread_create` returns `thread_exists`, read the returned thread.
  If its first post is yours, treat the create as delivered; otherwise post in
  the thread.
- Set `agent_continues` honestly. If you post an interim report and keep going,
  pass `true`; your final post on the work must pass `false`, or the work stays
  in your queue and the nudges keep coming.
- Do not post progress chatter. Post results, blockers, questions, and interim
  reports the user needs while long work continues.
- One consolidated reply is better than several fragments.
- Include what changed, how it was verified, and any remaining risk.
- If you need an answer, ask at most one focused question with
  `agent_continues=false` and leave the thread unresolved. The app will show it as awaiting the user.

## Resolution

- Resolve only when the request is finished and no answer from the user is
  needed. Resolving does not post a message. Pass the id of your final post as
  `after`.
- Do not resolve a thread whose last post is from the user. Reply first.
- You cannot resolve a thread whose last post is yours with
  `agent_continues=true` (`agent_continues` error). Post the final result with
  `false`, then resolve.
- The user can resolve a thread you are continuing. Treat that as a
  cancellation: it leaves your queue, so stop that work.
- The app automatically reopens a resolved thread when the user posts in it.
  You do not need to reopen it for an additional request.
- If a thread is already resolved but you decide more work is needed (for
  example, you need to ask a follow-up question), call
  `braydr_thread_unresolve` first, then post. `already_unresolved` means it is
  already open; just post.
- If `braydr_thread_create_post` fails with `thread_resolved`, re-read the
  thread. If the work is still needed, unresolve, post, and resolve again. If
  the user's resolution was a cancellation, stop.
- If `braydr_thread_resolve` returns `already_resolved`, the thread is already
  closed; treat it as done.

## Tools

- `braydr_channel_get` — channel name, counts, last main post, and
  `main_awaiting_agent`.
- `braydr_channel_list_posts` — main timeline, oldest to newest per page.
- `braydr_thread_get` — thread metadata and parent message.
- `braydr_thread_list_posts` — parent message, thread state, and replies.
- `braydr_message_get` — one message.
- `braydr_message_search` — full-text search across the main timeline and all
  threads, newest first. Use it to find past decisions and context.
- `braydr_thread_list` — filter: `unresolved`, `awaiting_agent`,
  `awaiting_user`, `resolved`, `all`. Paginated.
- `braydr_channel_create_post` — main timeline post. Requires
  `agent_continues`.
- `braydr_thread_create_post` — thread reply. Requires `agent_continues`.
- `braydr_thread_create` — start a thread and post its first reply. Requires
  `agent_continues`.
- `braydr_thread_resolve` / `braydr_thread_unresolve`.

Prefer `awaiting_agent` over scanning all unresolved threads when you need your
queue.
