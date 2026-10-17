-- Tunewire online features: accounts, shared & collaborative playlists,
-- follows, friend activity, shared Wrapped. Audio never touches the server:
-- only song info (title, artist, links) is stored.
--
-- Row level security decides who can read/change what; the app only ever
-- uses the public anon key plus the signed-in user's own token.

create extension if not exists pgcrypto;

-- Profiles ------------------------------------------------------------------
create table public.profiles (
  id uuid primary key references auth.users on delete cascade,
  name text not null default 'Listener' check (char_length(name) between 1 and 40),
  avatar text check (avatar is null or char_length(avatar) < 1000),
  share_activity boolean not null default true,
  created_at timestamptz not null default now()
);

-- New accounts get a profile from their Discord/Google name and picture.
create function public.handle_new_user() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  insert into public.profiles (id, name, avatar)
  values (
    new.id,
    left(coalesce(nullif(new.raw_user_meta_data->>'full_name', ''), nullif(new.raw_user_meta_data->>'name', ''), nullif(new.raw_user_meta_data->>'user_name', ''), 'Listener'), 40),
    new.raw_user_meta_data->>'avatar_url'
  );
  return new;
end $$;

create trigger on_auth_user_created after insert on auth.users
  for each row execute function public.handle_new_user();

alter table public.profiles enable row level security;
create policy "profiles are public" on public.profiles for select using (true);
create policy "edit own profile" on public.profiles for update using (id = auth.uid()) with check (id = auth.uid());

-- Shared playlists --------------------------------------------------------------
create table public.playlists (
  id uuid primary key default gen_random_uuid(),
  owner uuid not null default auth.uid() references public.profiles on delete cascade,
  title text not null check (char_length(title) between 1 and 120),
  description text not null default '' check (char_length(description) <= 600),
  cover text check (cover is null or char_length(cover) < 400000),
  tracks jsonb not null default '[]'::jsonb check (jsonb_typeof(tracks) = 'array' and octet_length(tracks::text) < 3000000),
  -- public: on your profile; unlisted: anyone with the link; private: you and members.
  visibility text not null default 'unlisted' check (visibility in ('public', 'unlisted', 'private')),
  collaborative boolean not null default false,
  invite_code text not null unique default encode(gen_random_bytes(12), 'hex'),
  version integer not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index playlists_owner on public.playlists (owner);

create table public.playlist_members (
  playlist_id uuid not null references public.playlists on delete cascade,
  user_id uuid not null default auth.uid() references public.profiles on delete cascade,
  -- editor: can change a collaborative playlist; follower: gets its updates.
  role text not null check (role in ('editor', 'follower')),
  joined_at timestamptz not null default now(),
  primary key (playlist_id, user_id)
);
create index playlist_members_user on public.playlist_members (user_id);

-- Helpers (security definer so policies don't recurse into each other).
create function public.is_member(pid uuid, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (select 1 from playlist_members where playlist_id = pid and user_id = uid)
$$;

create function public.can_edit(pid uuid, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (
    select 1 from playlists p where p.id = pid and (
      p.owner = uid
      or (p.collaborative and exists (select 1 from playlist_members m where m.playlist_id = pid and m.user_id = uid and m.role = 'editor'))
    )
  )
$$;

alter table public.playlists enable row level security;
create policy "read shared playlists" on public.playlists for select
  using (visibility in ('public', 'unlisted') or owner = auth.uid() or public.is_member(id, auth.uid()));
create policy "create own playlists" on public.playlists for insert with check (owner = auth.uid());
create policy "owner updates" on public.playlists for update using (owner = auth.uid()) with check (owner = auth.uid());
create policy "owner deletes" on public.playlists for delete using (owner = auth.uid());

alter table public.playlist_members enable row level security;
create policy "see memberships" on public.playlist_members for select
  using (user_id = auth.uid() or exists (select 1 from public.playlists p where p.id = playlist_id and p.owner = auth.uid()) or public.is_member(playlist_id, auth.uid()));
create policy "follow readable playlists" on public.playlist_members for insert
  with check (user_id = auth.uid() and role = 'follower'
    and exists (select 1 from public.playlists p where p.id = playlist_id and p.visibility in ('public', 'unlisted')));
create policy "leave or remove" on public.playlist_members for delete
  using (user_id = auth.uid() or exists (select 1 from public.playlists p where p.id = playlist_id and p.owner = auth.uid()));

-- Join a collaborative playlist with its invite code.
create function public.join_playlist(code text) returns uuid
language plpgsql security definer set search_path = public as $$
declare pid uuid;
begin
  if auth.uid() is null then raise exception 'sign in first'; end if;
  select id into pid from playlists where invite_code = code and collaborative;
  if pid is null then raise exception 'invite not valid'; end if;
  insert into playlist_members (playlist_id, user_id, role) values (pid, auth.uid(), 'editor')
    on conflict (playlist_id, user_id) do update set role = 'editor';
  return pid;
end $$;

-- Change a playlist's songs atomically, so editors never overwrite each other.
-- op: {"add": [tracks], "at": index?} | {"remove": [keys]} | {"move": [keys], "to": index}
--   | {"replace": [tracks]} (owner sync)
create function public.playlist_op(pid uuid, op jsonb) returns jsonb
language plpgsql security definer set search_path = public as $$
declare
  cur jsonb;
  ver integer;
  result jsonb;
  keys text[];
  moving jsonb;
  rest jsonb;
  pos integer;
begin
  if auth.uid() is null or not public.can_edit(pid, auth.uid()) then raise exception 'not allowed'; end if;
  select tracks, version into cur, ver from playlists where id = pid for update;

  if op ? 'replace' then
    if (select owner from playlists where id = pid) <> auth.uid() then raise exception 'only the owner can replace'; end if;
    result := op->'replace';
  elsif op ? 'add' then
    -- New songs (by key) appended or inserted, stamped with who/when.
    select coalesce(jsonb_agg(t || jsonb_build_object('_by', auth.uid(), '_at', (extract(epoch from now()) * 1000)::bigint)), '[]'::jsonb)
      into moving
      from jsonb_array_elements(op->'add') t
      where not exists (select 1 from jsonb_array_elements(cur) c where c->>'key' = t->>'key');
    pos := coalesce((op->>'at')::integer, jsonb_array_length(cur));
    select coalesce(jsonb_agg(e order by ord), '[]'::jsonb) into result from (
      select e, case when n <= pos then n else n + 100000 end as ord from jsonb_array_elements(cur) with ordinality as x(e, n)
      union all
      select e, pos + n * 0.0001 + 0.5 from jsonb_array_elements(moving) with ordinality as y(e, n)
    ) s;
  elsif op ? 'remove' then
    keys := array(select jsonb_array_elements_text(op->'remove'));
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into result
      from jsonb_array_elements(cur) with ordinality as x(e, n) where not (e->>'key' = any(keys));
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
  else
    raise exception 'unknown op';
  end if;

  update playlists set tracks = result, version = ver + 1, updated_at = now() where id = pid;
  return jsonb_build_object('version', ver + 1, 'count', jsonb_array_length(result));
end $$;

-- Follows -------------------------------------------------------------------
create table public.follows (
  follower uuid not null default auth.uid() references public.profiles on delete cascade,
  followee uuid not null references public.profiles on delete cascade,
  created_at timestamptz not null default now(),
  primary key (follower, followee),
  check (follower <> followee)
);
create index follows_followee on public.follows (followee);
alter table public.follows enable row level security;
create policy "follows are public" on public.follows for select using (true);
create policy "follow as yourself" on public.follows for insert with check (follower = auth.uid());
create policy "unfollow as yourself" on public.follows for delete using (follower = auth.uid());

-- Friend activity -----------------------------------------------------------
create table public.now_playing (
  user_id uuid primary key default auth.uid() references public.profiles on delete cascade,
  track jsonb check (track is null or octet_length(track::text) < 20000),
  playing boolean not null default false,
  updated_at timestamptz not null default now()
);
alter table public.now_playing enable row level security;
-- Visible to the people who follow you (unless you turned sharing off).
create policy "followers see activity" on public.now_playing for select using (
  user_id = auth.uid() or (
    exists (select 1 from public.follows f where f.follower = auth.uid() and f.followee = user_id)
    and exists (select 1 from public.profiles p where p.id = user_id and p.share_activity)
  )
);
create policy "write own activity" on public.now_playing for insert with check (user_id = auth.uid());
create policy "update own activity" on public.now_playing for update using (user_id = auth.uid()) with check (user_id = auth.uid());
create policy "clear own activity" on public.now_playing for delete using (user_id = auth.uid());

-- Shared Wrapped ------------------------------------------------------------
create table public.wrapped (
  user_id uuid not null default auth.uid() references public.profiles on delete cascade,
  year integer not null check (year between 2020 and 2100),
  data jsonb not null check (octet_length(data::text) < 50000),
  updated_at timestamptz not null default now(),
  primary key (user_id, year)
);
alter table public.wrapped enable row level security;
create policy "followers and self see wrapped" on public.wrapped for select using (
  user_id = auth.uid() or exists (select 1 from public.follows f where f.follower = auth.uid() and f.followee = user_id)
  or exists (select 1 from public.follows f where f.followee = auth.uid() and f.follower = user_id)
);
create policy "write own wrapped" on public.wrapped for insert with check (user_id = auth.uid());
create policy "update own wrapped" on public.wrapped for update using (user_id = auth.uid()) with check (user_id = auth.uid());

-- App info (latest versions, announcements) -----------------------------------
create table public.app_meta (
  key text primary key,
  value jsonb not null,
  updated_at timestamptz not null default now()
);
alter table public.app_meta enable row level security;
create policy "app info is public" on public.app_meta for select using (true);

-- Live updates (collaborative playlists, friend activity, update pushes).
alter publication supabase_realtime add table public.playlists, public.now_playing, public.app_meta;
