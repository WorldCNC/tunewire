-- Tunewire online features, v2: security and privacy fixes plus new social
-- features.
--
--  * Invite codes for collaborative playlists live in their own table that
--    only the owner can read (before, anyone who could open a playlist could
--    read its code and make themselves an editor). Existing codes are rotated.
--  * Accounts are Discord only: guest (anonymous) sessions can read what's
--    public but can't write anything.
--  * Following needs the other person's approval. What you're playing and
--    your Wrapped are only visible to approved followers, and sharing what
--    you play is off until you turn it on (everyone is asked again).
--  * Listen Together sessions are rows the server checks: only the host can
--    change what plays, members are tracked, the host can kick, hand over,
--    approve suggestions; chat and skip votes are stored per session.
--  * Notifications (follow requests, accepted follows, playlist activity,
--    friends starting a session).
--  * Collaborative edits are published as small change rows instead of the
--    whole playlist (up to 3 MB) going to every listener on every edit.
--  * Limits against spam (playlists per hour, session events per minute) and
--    a rate-limit table for the error-report Edge Function.

-- Who counts as signed in: a real (Discord) account, not an anonymous session.
create function public.signed_in() returns boolean
language sql stable set search_path = public as $$
  select auth.uid() is not null and coalesce((auth.jwt() ->> 'is_anonymous')::boolean, false) = false
$$;

-- Server clock (clients line up Listen Together positions with it).
create function public.server_now() returns double precision
language sql stable as $$ select extract(epoch from clock_timestamp()) * 1000 $$;

-- Invite codes ------------------------------------------------------------------
create table public.playlist_invites (
  playlist_id uuid primary key references public.playlists on delete cascade,
  code text not null unique default encode(extensions.gen_random_bytes(16), 'hex'),
  created_at timestamptz not null default now()
);
-- Every existing collaborative playlist gets a new code (old links stop working).
insert into public.playlist_invites (playlist_id) select id from public.playlists where collaborative;
alter table public.playlists drop column invite_code;
alter table public.playlist_invites enable row level security;
-- No policies: only the functions below (security definer) touch this table.
revoke all on public.playlist_invites from anon, authenticated;

-- The owner's invite code (made on first use).
create function public.playlist_invite(pid uuid) returns text
language plpgsql security definer set search_path = public as $$
declare c text;
begin
  if not public.signed_in() or not exists (select 1 from playlists where id = pid and owner = auth.uid()) then
    raise exception 'not allowed';
  end if;
  insert into playlist_invites (playlist_id) values (pid) on conflict (playlist_id) do nothing;
  select code into c from playlist_invites where playlist_id = pid;
  return c;
end $$;

-- A new code: the old invite link stops working.
create function public.rotate_playlist_invite(pid uuid) returns text
language plpgsql security definer set search_path = public as $$
declare c text;
begin
  if not public.signed_in() or not exists (select 1 from playlists where id = pid and owner = auth.uid()) then
    raise exception 'not allowed';
  end if;
  insert into playlist_invites (playlist_id) values (pid)
    on conflict (playlist_id) do update set code = encode(extensions.gen_random_bytes(16), 'hex'), created_at = now();
  select code into c from playlist_invites where playlist_id = pid;
  return c;
end $$;

create or replace function public.join_playlist(code text) returns uuid
language plpgsql security definer set search_path = public as $$
declare pid uuid;
begin
  if not public.signed_in() then raise exception 'sign in first'; end if;
  select i.playlist_id into pid from playlist_invites i join playlists p on p.id = i.playlist_id
    where i.code = join_playlist.code and p.collaborative;
  if pid is null then raise exception 'invite not valid'; end if;
  insert into playlist_members (playlist_id, user_id, role) values (pid, auth.uid(), 'editor')
    on conflict (playlist_id, user_id) do update set role = 'editor';
  return pid;
end $$;

-- Writes need a real account ----------------------------------------------------------
drop policy "edit own profile" on public.profiles;
create policy "edit own profile" on public.profiles for update
  using (id = auth.uid() and public.signed_in()) with check (id = auth.uid());

drop policy "create own playlists" on public.playlists;
drop policy "owner updates" on public.playlists;
drop policy "owner deletes" on public.playlists;
create policy "create own playlists" on public.playlists for insert with check (owner = auth.uid() and public.signed_in());
create policy "owner updates" on public.playlists for update using (owner = auth.uid() and public.signed_in()) with check (owner = auth.uid());
create policy "owner deletes" on public.playlists for delete using (owner = auth.uid() and public.signed_in());

drop policy "follow readable playlists" on public.playlist_members;
create policy "follow readable playlists" on public.playlist_members for insert
  with check (user_id = auth.uid() and role = 'follower' and public.signed_in()
    and exists (select 1 from public.playlists p where p.id = playlist_id and p.visibility in ('public', 'unlisted')));

-- Spam limits: at most 300 shared playlists, 60 new ones an hour.
create function public.limit_playlists() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if (select count(*) from playlists where owner = new.owner) >= 300 then
    raise exception 'too many shared playlists (300 max)';
  end if;
  if (select count(*) from playlists where owner = new.owner and created_at > now() - interval '1 hour') >= 60 then
    raise exception 'too many new playlists, try again later';
  end if;
  return new;
end $$;
create trigger limit_playlists before insert on public.playlists for each row execute function public.limit_playlists();

-- Small change rows instead of whole playlists ------------------------------------------
create table public.playlist_changes (
  id bigint generated always as identity primary key,
  playlist_id uuid not null references public.playlists on delete cascade,
  version integer not null,
  -- {"add": [...], "at": n} | {"remove": [keys]} | {"move": [keys], "to": n}
  -- | {"replace": true} | {"meta": {...}}: clients refetch for replace.
  op jsonb not null,
  by uuid references public.profiles on delete set null,
  created_at timestamptz not null default now()
);
create index playlist_changes_pl on public.playlist_changes (playlist_id, id);
alter table public.playlist_changes enable row level security;

create function public.can_read_playlist(pid uuid, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (select 1 from playlists p where p.id = pid and (p.visibility in ('public', 'unlisted') or p.owner = uid))
    or public.is_member(pid, uid)
$$;
create policy "read changes of readable playlists" on public.playlist_changes for select
  using (public.can_read_playlist(playlist_id, auth.uid()));

-- Title, description, cover, visibility changes → one small change row.
create function public.playlist_meta_changed() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if new.title is distinct from old.title or new.description is distinct from old.description
     or new.cover is distinct from old.cover or new.visibility is distinct from old.visibility
     or new.collaborative is distinct from old.collaborative then
    insert into playlist_changes (playlist_id, version, op, by) values (new.id, new.version, jsonb_build_object('meta', jsonb_build_object(
      'title', new.title, 'description', new.description, 'visibility', new.visibility, 'collaborative', new.collaborative,
      'cover_changed', new.cover is distinct from old.cover)), auth.uid());
  end if;
  return new;
end $$;
create trigger playlist_meta_changed after update on public.playlists for each row execute function public.playlist_meta_changed();

-- Notifications ------------------------------------------------------------------------
create table public.notifications (
  id bigint generated always as identity primary key,
  user_id uuid not null references public.profiles on delete cascade,
  kind text not null check (kind in ('follow_request', 'follow_accepted', 'playlist_activity', 'jam_started')),
  actor uuid references public.profiles on delete cascade,
  data jsonb not null default '{}'::jsonb check (octet_length(data::text) < 4000),
  read boolean not null default false,
  created_at timestamptz not null default now()
);
create index notifications_user on public.notifications (user_id, created_at desc);
alter table public.notifications enable row level security;
create policy "see own notifications" on public.notifications for select using (user_id = auth.uid());
create policy "mark own notifications" on public.notifications for update using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy "clear own notifications" on public.notifications for delete using (user_id = auth.uid());
revoke update on public.notifications from anon, authenticated;
grant update (read) on public.notifications to authenticated;

create function public.notify(target uuid, k text, who uuid, d jsonb) returns void
language plpgsql security definer set search_path = public as $$
begin
  if target is null or target = who then return; end if;
  -- Playlist activity: one unread note per playlist and person an hour.
  if k = 'playlist_activity' and exists (
    select 1 from notifications where user_id = target and kind = k and actor = who and not read
      and data->>'playlist' = d->>'playlist' and created_at > now() - interval '1 hour') then
    return;
  end if;
  insert into notifications (user_id, kind, actor, data) values (target, k, who, d);
  delete from notifications where user_id = target and created_at < now() - interval '60 days';
end $$;
revoke execute on function public.notify(uuid, text, uuid, jsonb) from public, anon, authenticated;

-- Follows need approval ------------------------------------------------------------------
alter table public.follows add column status text not null default 'pending' check (status in ('pending', 'accepted'));
-- Follows made before approval existed stay as they were.
update public.follows set status = 'accepted';
create index follows_pending on public.follows (followee) where status = 'pending';

drop policy "follows are public" on public.follows;
drop policy "follow as yourself" on public.follows;
drop policy "unfollow as yourself" on public.follows;
-- Accepted follows are public (follower counts); requests only to the two people.
create policy "see follows" on public.follows for select
  using (status = 'accepted' or follower = auth.uid() or followee = auth.uid());
create policy "ask to follow" on public.follows for insert
  with check (follower = auth.uid() and status = 'pending' and public.signed_in());
create policy "accept followers" on public.follows for update
  using (followee = auth.uid() and public.signed_in()) with check (followee = auth.uid() and status = 'accepted');
-- Unfollow, cancel a request, or remove a follower.
create policy "end a follow" on public.follows for delete
  using (follower = auth.uid() or followee = auth.uid());
revoke update on public.follows from anon, authenticated;
grant update (status) on public.follows to authenticated;

create function public.follow_notify() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if tg_op = 'INSERT' then
    perform public.notify(new.followee, 'follow_request', new.follower, '{}'::jsonb);
  elsif new.status = 'accepted' and old.status = 'pending' then
    perform public.notify(new.follower, 'follow_accepted', new.followee, '{}'::jsonb);
    -- The request note is handled.
    update notifications set read = true where user_id = new.followee and kind = 'follow_request' and actor = new.follower;
  end if;
  return new;
end $$;
create trigger follow_notify after insert or update on public.follows for each row execute function public.follow_notify();

create function public.follows_accepted(a uuid, b uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (select 1 from follows where follower = a and followee = b and status = 'accepted')
$$;

-- Sharing what you play is opt-in (asked again on next sign-in).
alter table public.profiles alter column share_activity set default false;
update public.profiles set share_activity = false;
-- Whether approved followers can compare Wrapped with you.
alter table public.profiles add column share_wrapped boolean not null default true;

drop policy "followers see activity" on public.now_playing;
create policy "followers see activity" on public.now_playing for select using (
  user_id = auth.uid() or (
    public.follows_accepted(auth.uid(), user_id)
    and exists (select 1 from public.profiles p where p.id = user_id and p.share_activity)
  )
);
drop policy "write own activity" on public.now_playing;
drop policy "update own activity" on public.now_playing;
create policy "write own activity" on public.now_playing for insert with check (user_id = auth.uid() and public.signed_in());
create policy "update own activity" on public.now_playing for update using (user_id = auth.uid() and public.signed_in()) with check (user_id = auth.uid());

drop policy "followers and self see wrapped" on public.wrapped;
create policy "approved friends see wrapped" on public.wrapped for select using (
  user_id = auth.uid() or (
    (public.follows_accepted(auth.uid(), user_id) or public.follows_accepted(user_id, auth.uid()))
    and exists (select 1 from public.profiles p where p.id = user_id and p.share_wrapped)
  )
);
drop policy "write own wrapped" on public.wrapped;
drop policy "update own wrapped" on public.wrapped;
create policy "write own wrapped" on public.wrapped for insert with check (user_id = auth.uid() and public.signed_in());
create policy "update own wrapped" on public.wrapped for update using (user_id = auth.uid() and public.signed_in()) with check (user_id = auth.uid());
create policy "delete own wrapped" on public.wrapped for delete using (user_id = auth.uid());

-- Collaborative edits: change rows + activity notes ------------------------------------------
create or replace function public.playlist_op(pid uuid, op jsonb) returns jsonb
language plpgsql security definer set search_path = public as $$
declare
  cur jsonb;
  ver integer;
  result jsonb;
  keys text[];
  moving jsonb;
  rest jsonb;
  pos integer;
  published jsonb;
  pl record;
begin
  if not public.signed_in() or not public.can_edit(pid, auth.uid()) then raise exception 'not allowed'; end if;
  select * into pl from playlists where id = pid for update;
  cur := pl.tracks;
  ver := pl.version;

  if op ? 'replace' then
    if pl.owner <> auth.uid() then raise exception 'only the owner can replace'; end if;
    result := op->'replace';
    published := jsonb_build_object('replace', true);
  elsif op ? 'add' then
    -- New songs (by key) appended or inserted, stamped with who/when.
    select coalesce(jsonb_agg(t || jsonb_build_object('_by', auth.uid(), '_at', (extract(epoch from now()) * 1000)::bigint)), '[]'::jsonb)
      into moving
      from jsonb_array_elements(op->'add') t
      where not exists (select 1 from jsonb_array_elements(cur) c where c->>'key' = t->>'key');
    pos := least(greatest(coalesce((op->>'at')::integer, jsonb_array_length(cur)), 0), jsonb_array_length(cur));
    select coalesce(jsonb_agg(e order by ord), '[]'::jsonb) into result from (
      select e, case when n <= pos then n else n + 100000 end as ord from jsonb_array_elements(cur) with ordinality as x(e, n)
      union all
      select e, pos + n * 0.0001 + 0.5 from jsonb_array_elements(moving) with ordinality as y(e, n)
    ) s;
    published := jsonb_build_object('add', moving, 'at', pos);
  elsif op ? 'remove' then
    keys := array(select jsonb_array_elements_text(op->'remove'));
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into result
      from jsonb_array_elements(cur) with ordinality as x(e, n) where not (e->>'key' = any(keys));
    published := jsonb_build_object('remove', op->'remove');
  elsif op ? 'move' then
    keys := array(select jsonb_array_elements_text(op->'move'));
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into moving from jsonb_array_elements(cur) with ordinality as x(e, n) where e->>'key' = any(keys);
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into rest from jsonb_array_elements(cur) with ordinality as x(e, n) where not (e->>'key' = any(keys));
    pos := least(greatest(coalesce((op->>'to')::integer, 0), 0), jsonb_array_length(rest));
    select coalesce(jsonb_agg(e order by ord), '[]'::jsonb) into result from (
      select e, case when n <= pos then n else n + 100000 end as ord from jsonb_array_elements(rest) with ordinality as x(e, n)
      union all
      select e, pos + n * 0.0001 + 0.5 from jsonb_array_elements(moving) with ordinality as y(e, n)
    ) s;
    published := jsonb_build_object('move', op->'move', 'to', pos);
  else
    raise exception 'unknown op';
  end if;

  update playlists set tracks = result, version = ver + 1, updated_at = now() where id = pid;
  insert into playlist_changes (playlist_id, version, op, by) values (pid, ver + 1, published, auth.uid());
  delete from playlist_changes where playlist_id = pid and created_at < now() - interval '7 days';

  -- Tell the owner and the other editors (at most once an hour per person).
  if pl.collaborative and not (op ? 'replace') then
    perform public.notify(r.uid, 'playlist_activity', auth.uid(), jsonb_build_object('playlist', pid, 'title', pl.title))
      from (select pl.owner as uid union select user_id from playlist_members where playlist_id = pid and role = 'editor') r;
  end if;
  return jsonb_build_object('version', ver + 1, 'count', jsonb_array_length(result));
end $$;

-- Listen Together ------------------------------------------------------------------------------
create table public.jams (
  code text primary key check (code ~ '^[a-z0-9]{8,24}$'),
  -- Who controls playback now (can be handed over).
  host uuid not null default auth.uid() references public.profiles on delete cascade,
  -- Whose room it is (a persistent room keeps its code).
  owner uuid not null default auth.uid() references public.profiles on delete cascade,
  title text not null default '' check (char_length(title) <= 80),
  persistent boolean not null default false,
  active boolean not null default true,
  friends_can_join boolean not null default false,
  approve_adds boolean not null default true,
  -- {track, playing, position, upcoming}; state_at is the server's clock.
  state jsonb check (state is null or octet_length(state::text) < 20000),
  state_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create unique index jams_one_room on public.jams (owner) where persistent;

create table public.jam_members (
  code text not null references public.jams on delete cascade,
  user_id uuid not null default auth.uid() references public.profiles on delete cascade,
  joined_at timestamptz not null default now(),
  primary key (code, user_id)
);
create index jam_members_user on public.jam_members (user_id);

create table public.jam_bans (
  code text not null references public.jams on delete cascade,
  user_id uuid not null references public.profiles on delete cascade,
  primary key (code, user_id)
);

create table public.jam_events (
  id bigint generated always as identity primary key,
  code text not null references public.jams on delete cascade,
  user_id uuid not null default auth.uid() references public.profiles on delete cascade,
  -- suggest: {track}; chat: {text}; vote_skip: {key}
  kind text not null check (kind in ('suggest', 'chat', 'vote_skip')),
  payload jsonb not null default '{}'::jsonb check (octet_length(payload::text) < 8000),
  -- Suggestions: pending → approved | rejected (by the host).
  status text check (status is null or status in ('pending', 'approved', 'rejected')),
  created_at timestamptz not null default now()
);
create index jam_events_code on public.jam_events (code, id);

create function public.is_jam_member(c text, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (select 1 from jam_members where code = c and user_id = uid)
    or exists (select 1 from jams where code = c and (host = uid or owner = uid))
$$;

alter table public.jams enable row level security;
alter table public.jam_members enable row level security;
alter table public.jam_bans enable row level security;
alter table public.jam_events enable row level security;

create policy "members see the session" on public.jams for select using (public.is_jam_member(code, auth.uid()));
create policy "start a session" on public.jams for insert
  with check (host = auth.uid() and owner = auth.uid() and public.signed_in());
-- Only the host changes what plays (and the session's options).
create policy "host controls" on public.jams for update
  using (host = auth.uid() and public.signed_in()) with check (host = auth.uid());
create policy "owner ends" on public.jams for delete using (owner = auth.uid() or host = auth.uid());
revoke update on public.jams from anon, authenticated;
grant update (state, active, title, approve_adds, friends_can_join) on public.jams to authenticated;

create policy "members see members" on public.jam_members for select using (public.is_jam_member(code, auth.uid()));
create policy "leave a session" on public.jam_members for delete using (user_id = auth.uid());

create policy "host sees bans" on public.jam_bans for select
  using (exists (select 1 from public.jams j where j.code = jam_bans.code and (j.host = auth.uid() or j.owner = auth.uid())));

create policy "members see events" on public.jam_events for select using (public.is_jam_member(code, auth.uid()));
create policy "members post events" on public.jam_events for insert
  with check (user_id = auth.uid() and public.signed_in() and public.is_jam_member(code, auth.uid())
    and (kind <> 'suggest' or status = 'pending') and (kind = 'suggest' or status is null));
create policy "host answers suggestions" on public.jam_events for update
  using (kind = 'suggest' and exists (select 1 from public.jams j where j.code = jam_events.code and j.host = auth.uid()))
  with check (status in ('approved', 'rejected'));
revoke update on public.jam_events from anon, authenticated;
grant update (status) on public.jam_events to authenticated;

-- The server stamps when the state was set (one clock for everyone).
create function public.jam_touch() returns trigger
language plpgsql as $$
begin
  if new.state is distinct from old.state then new.state_at := clock_timestamp(); end if;
  new.updated_at := now();
  return new;
end $$;
create trigger jam_touch before update on public.jams for each row execute function public.jam_touch();

-- Chat/vote/suggest limits: 20 a minute per person; text and old events trimmed.
create function public.jam_event_guard() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if (select count(*) from jam_events where code = new.code and user_id = new.user_id and created_at > now() - interval '1 minute') >= 20 then
    raise exception 'slow down a little';
  end if;
  if new.kind = 'chat' then
    if char_length(coalesce(new.payload->>'text', '')) not between 1 and 300 then raise exception 'messages are 1-300 characters'; end if;
    new.payload := jsonb_build_object('text', new.payload->>'text');
  end if;
  if new.kind = 'vote_skip' then
    new.payload := jsonb_build_object('key', left(coalesce(new.payload->>'key', ''), 300));
  end if;
  delete from jam_events where code = new.code and created_at < now() - interval '1 day';
  return new;
end $$;
create trigger jam_event_guard before insert on public.jam_events for each row execute function public.jam_event_guard();

-- Start (or reopen your persistent room). Returns the session's code.
create function public.open_jam(new_code text, make_persistent boolean default false, room_title text default '', friends boolean default false)
returns text
language plpgsql security definer set search_path = public as $$
declare c text;
begin
  if not public.signed_in() then raise exception 'sign in first'; end if;
  -- Sessions nobody used for a day are cleaned up.
  delete from jams where not persistent and updated_at < now() - interval '1 day';
  if make_persistent then
    select code into c from jams where owner = auth.uid() and persistent;
  end if;
  if c is null then
    insert into jams (code, host, owner, title, persistent, friends_can_join)
      values (new_code, auth.uid(), auth.uid(), left(coalesce(room_title, ''), 80), make_persistent, friends)
      returning code into c;
  else
    update jams set active = true, host = auth.uid(), state = null, friends_can_join = friends,
      title = case when room_title <> '' then left(room_title, 80) else title end
      where code = c;
    delete from jam_members where code = c;
  end if;
  insert into jam_members (code, user_id) values (c, auth.uid()) on conflict do nothing;
  if friends then
    perform public.notify(f.follower, 'jam_started', auth.uid(), jsonb_build_object('code', c, 'title', room_title))
      from follows f where f.followee = auth.uid() and f.status = 'accepted';
  end if;
  return c;
end $$;

create function public.join_jam(jam_code text) returns jsonb
language plpgsql security definer set search_path = public as $$
declare j jams;
begin
  if not public.signed_in() then raise exception 'sign in first'; end if;
  select * into j from jams where code = jam_code;
  if j is null or not j.active then raise exception 'this session has ended'; end if;
  if exists (select 1 from jam_bans where code = jam_code and user_id = auth.uid()) then
    raise exception 'the host removed you from this session';
  end if;
  insert into jam_members (code, user_id) values (jam_code, auth.uid()) on conflict do nothing;
  return to_jsonb(j);
end $$;

create function public.jam_handoff(jam_code text, new_host uuid) returns void
language plpgsql security definer set search_path = public as $$
begin
  if not exists (select 1 from jams where code = jam_code and host = auth.uid()) then raise exception 'only the host can do that'; end if;
  if not exists (select 1 from jam_members where code = jam_code and user_id = new_host) then raise exception 'they are not in the session'; end if;
  update jams set host = new_host, updated_at = now() where code = jam_code;
end $$;

create function public.jam_kick(jam_code text, who uuid) returns void
language plpgsql security definer set search_path = public as $$
begin
  if not exists (select 1 from jams where code = jam_code and host = auth.uid()) then raise exception 'only the host can do that'; end if;
  if who = auth.uid() then raise exception 'you can''t remove yourself'; end if;
  delete from jam_members where code = jam_code and user_id = who;
  insert into jam_bans (code, user_id) values (jam_code, who) on conflict do nothing;
end $$;

-- End: a persistent room goes quiet (keeps its code), others are deleted.
create function public.end_jam(jam_code text) returns void
language plpgsql security definer set search_path = public as $$
begin
  if not exists (select 1 from jams where code = jam_code and (host = auth.uid() or owner = auth.uid())) then raise exception 'only the host can do that'; end if;
  delete from jams where code = jam_code and not persistent;
  update jams set active = false, state = null where code = jam_code and persistent;
  delete from jam_members where code = jam_code;
  delete from jam_events where code = jam_code;
  delete from jam_bans where code = jam_code;
end $$;

-- Presence and reactions use a private Realtime channel per session
-- ("jam:<code>"): only members can join it.
do $$
begin
  if exists (select 1 from information_schema.tables where table_schema = 'realtime' and table_name = 'messages') then
    execute $p$create policy "jam members receive" on realtime.messages for select to authenticated
      using (realtime.topic() like 'jam:%' and public.is_jam_member(substr(realtime.topic(), 5), auth.uid()))$p$;
    execute $p$create policy "jam members send" on realtime.messages for insert to authenticated
      with check (realtime.topic() like 'jam:%' and public.is_jam_member(substr(realtime.topic(), 5), auth.uid()))$p$;
  end if;
end $$;

-- Link previews (share Edge Function): a few fields, never the whole song list.
-- Security invoker: the caller's row level security applies.
create function public.playlist_preview(pid uuid)
returns table (title text, description text, owner_name text, songs integer, thumbnail text, has_cover boolean)
language sql stable security invoker set search_path = public as $$
  select p.title, p.description, pr.name, jsonb_array_length(p.tracks), p.tracks->0->>'thumbnail', p.cover is not null
  from playlists p join profiles pr on pr.id = p.owner where p.id = pid
$$;

create function public.playlist_cover(pid uuid) returns text
language sql stable security invoker set search_path = public as $$
  select coalesce(cover, tracks->0->>'thumbnail') from playlists where id = pid
$$;

-- Error reports: per-address limits kept for the Edge Function ----------------------------------
create table public.report_limits (
  bucket text primary key,
  count integer not null default 0,
  window_start timestamptz not null default now()
);
alter table public.report_limits enable row level security;
revoke all on public.report_limits from anon, authenticated;

create function public.report_allow(b text, max_count integer, window_secs integer) returns boolean
language plpgsql security definer set search_path = public as $$
declare n integer;
begin
  insert into report_limits (bucket, count, window_start) values (b, 1, now())
  on conflict (bucket) do update set
    count = case when report_limits.window_start < now() - make_interval(secs => window_secs) then 1 else report_limits.count + 1 end,
    window_start = case when report_limits.window_start < now() - make_interval(secs => window_secs) then now() else report_limits.window_start end
  returning count into n;
  return n <= max_count;
end $$;
revoke execute on function public.report_allow(text, integer, integer) from public, anon, authenticated;
grant execute on function public.report_allow(text, integer, integer) to service_role;

-- Realtime: small change rows instead of whole playlists -----------------------------------------
do $$
begin
  if exists (select 1 from pg_publication_tables where pubname = 'supabase_realtime' and schemaname = 'public' and tablename = 'playlists') then
    alter publication supabase_realtime drop table public.playlists;
  end if;
end $$;
alter publication supabase_realtime add table public.playlist_changes, public.notifications, public.jams, public.jam_events, public.jam_members;
