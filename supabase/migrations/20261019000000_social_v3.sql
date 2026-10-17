-- Social, part 3: richer profiles and friends.
--
--  * Profiles: bio, pronouns and the Discord avatar decoration (Discord CDN
--    only). "Share my listening" (off by default) shows top artists and
--    songs, recently played and weekly minutes to approved followers.
--  * Badges are worked out by the server from what it knows (no one can
--    give themselves one).
--  * Presence and live progress: what's playing now carries its position,
--    and an "online" time refreshed while the app is open.
--  * Send a song to a friend: an inbox with a note and a reaction.

alter table public.profiles
  add column bio text not null default '' check (char_length(bio) <= 300),
  add column pronouns text not null default '' check (char_length(pronouns) <= 40),
  add column decoration text check (decoration is null or decoration ~ '^https://cdn\.discordapp\.com/avatar-decoration-presets/(a_)?[0-9a-f]{16,64}\.png(\?size=[0-9]{2,4}(&passthrough=(true|false))?)?$'),
  add column share_stats boolean not null default false;

-- Live progress and presence ------------------------------------------------------------
alter table public.now_playing
  add column position double precision check (position is null or position between 0 and 86400),
  add column duration double precision check (duration is null or duration between 0 and 86400),
  add column online_at timestamptz;

-- Listening stats a user chose to share ------------------------------------------------
create table public.profile_stats (
  user_id uuid primary key default auth.uid() references public.profiles on delete cascade,
  -- [{name, thumbnail}] and [track] (top 5 of the last 4 weeks)
  top_artists jsonb not null default '[]'::jsonb check (jsonb_typeof(top_artists) = 'array' and octet_length(top_artists::text) < 20000),
  top_tracks jsonb not null default '[]'::jsonb check (jsonb_typeof(top_tracks) = 'array' and octet_length(top_tracks::text) < 40000),
  recent jsonb not null default '[]'::jsonb check (jsonb_typeof(recent) = 'array' and octet_length(recent::text) < 40000),
  week_start date,
  week_minutes integer not null default 0 check (week_minutes between 0 and 10080),
  month_minutes integer not null default 0 check (month_minutes between 0 and 43200),
  artists_count integer not null default 0 check (artists_count between 0 and 100000),
  streak integer not null default 0 check (streak between 0 and 10000),
  -- "night" | "morning" | "day" | "" (when they mostly listen)
  time_of_day text not null default '' check (time_of_day in ('', 'night', 'morning', 'day')),
  updated_at timestamptz not null default now()
);
alter table public.profile_stats enable row level security;
create policy "shared stats for approved followers" on public.profile_stats for select using (
  user_id = auth.uid() or (
    public.follows_accepted(auth.uid(), user_id)
    and exists (select 1 from public.profiles p where p.id = user_id and p.share_stats)
  )
);
create policy "write own stats" on public.profile_stats for insert with check (user_id = auth.uid() and public.signed_in());
create policy "update own stats" on public.profile_stats for update using (user_id = auth.uid() and public.signed_in()) with check (user_id = auth.uid());
create policy "delete own stats" on public.profile_stats for delete using (user_id = auth.uid());

-- Sending songs to friends ------------------------------------------------------------------
create table public.song_shares (
  id bigint generated always as identity primary key,
  from_user uuid not null default auth.uid() references public.profiles on delete cascade,
  to_user uuid not null references public.profiles on delete cascade,
  track jsonb not null check (octet_length(track::text) < 20000),
  note text not null default '' check (char_length(note) <= 200),
  read boolean not null default false,
  reaction text check (reaction is null or reaction in ('🔥', '❤️', '😂', '🕺', '👏', '😮')),
  created_at timestamptz not null default now(),
  check (from_user <> to_user)
);
create index song_shares_to on public.song_shares (to_user, created_at desc);
alter table public.song_shares enable row level security;
create policy "see songs sent to or by you" on public.song_shares for select using (to_user = auth.uid() or from_user = auth.uid());
-- Only to people you follow or who follow you (approved).
create policy "send songs to friends" on public.song_shares for insert with check (
  from_user = auth.uid() and public.signed_in()
  and (public.follows_accepted(auth.uid(), to_user) or public.follows_accepted(to_user, auth.uid()))
);
create policy "answer songs sent to you" on public.song_shares for update using (to_user = auth.uid()) with check (to_user = auth.uid());
create policy "clear songs sent to you" on public.song_shares for delete using (to_user = auth.uid());
revoke update on public.song_shares from anon, authenticated;
grant update (read, reaction) on public.song_shares to authenticated;

create function public.song_share_guard() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if (select count(*) from song_shares where from_user = new.from_user and created_at > now() - interval '1 hour') >= 30 then
    raise exception 'slow down a little';
  end if;
  new.note := btrim(new.note);
  return new;
end $$;
create trigger song_share_guard before insert on public.song_shares for each row execute function public.song_share_guard();

alter table public.notifications drop constraint notifications_kind_check;
alter table public.notifications add constraint notifications_kind_check
  check (kind in ('follow_request', 'follow_accepted', 'playlist_activity', 'jam_started', 'song_shared'));

create function public.song_share_notify() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  perform public.notify(new.to_user, 'song_shared', new.from_user,
    jsonb_build_object('share', new.id, 'title', left(coalesce(new.track->>'title', ''), 120), 'artist', left(coalesce(new.track->>'artist', ''), 120)));
  return new;
end $$;
create trigger song_share_notify after insert on public.song_shares for each row execute function public.song_share_notify();

-- Counters behind some badges ------------------------------------------------------------------
create table public.profile_counters (
  user_id uuid primary key references public.profiles on delete cascade,
  sessions_hosted integer not null default 0,
  songs_sent integer not null default 0
);
alter table public.profile_counters enable row level security;
revoke all on public.profile_counters from anon, authenticated;

create function public.count_hosted() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  insert into profile_counters (user_id, sessions_hosted) values (new.owner, 1)
    on conflict (user_id) do update set sessions_hosted = profile_counters.sessions_hosted + 1;
  return new;
end $$;
create trigger count_hosted after insert on public.jams for each row execute function public.count_hosted();
-- Reopening a room counts as hosting again.
create function public.count_reopened() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if new.active and not old.active then
    insert into profile_counters (user_id, sessions_hosted) values (new.owner, 1)
      on conflict (user_id) do update set sessions_hosted = profile_counters.sessions_hosted + 1;
  end if;
  return new;
end $$;
create trigger count_reopened after update of active on public.jams for each row execute function public.count_reopened();

create function public.count_sent() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  insert into profile_counters (user_id, songs_sent) values (new.from_user, 1)
    on conflict (user_id) do update set songs_sent = profile_counters.songs_sent + 1;
  return new;
end $$;
create trigger count_sent after insert on public.song_shares for each row execute function public.count_sent();

-- Badges ---------------------------------------------------------------------------------------
-- The developers' accounts: app_meta key "developers" = ["<user id>", …].
create function public.profile_badges(uid uuid) returns text[]
language plpgsql stable security definer set search_path = public as $$
declare
  b text[] := '{}';
  p profiles;
  s profile_stats;
  c profile_counters;
  followers integer;
  curated integer;
begin
  select * into p from profiles where id = uid;
  if p is null then return b; end if;
  select * into c from profile_counters where user_id = uid;
  if exists (select 1 from app_meta where key = 'developers' and value ? uid::text) then b := array_append(b, 'developer'); end if;
  if p.created_at < timestamptz '2027-01-01' then b := array_append(b, 'early'); end if;
  select count(*) into followers from follows where followee = uid and status = 'accepted';
  if followers >= 10 then b := array_append(b, 'social'); end if;
  select count(*) into curated from playlist_members m join playlists pl on pl.id = m.playlist_id
    where pl.owner = uid and m.role = 'follower';
  if curated >= 10 then b := array_append(b, 'curator'); end if;
  if coalesce(c.sessions_hosted, 0) >= 10 then b := array_append(b, 'host'); end if;
  if coalesce(c.songs_sent, 0) >= 25 then b := array_append(b, 'connector'); end if;
  -- Listening badges only for people who share their listening.
  if p.share_stats then
    select * into s from profile_stats where user_id = uid;
    if s.streak >= 30 then b := array_append(b, 'streak30'); elsif s.streak >= 7 then b := array_append(b, 'streak7'); end if;
    if s.time_of_day = 'night' then b := array_append(b, 'night_owl'); elsif s.time_of_day = 'morning' then b := array_append(b, 'early_bird'); end if;
    if s.artists_count >= 100 then b := array_append(b, 'explorer'); end if;
  end if;
  return b;
end $$;

-- Weekly leaderboard: you and the people you follow who share their listening.
create function public.friend_leaderboard() returns table (user_id uuid, name text, avatar text, decoration text, week_minutes integer, streak integer)
language sql stable security definer set search_path = public as $$
  select p.id, p.name, p.avatar, p.decoration,
    case when s.week_start = date_trunc('week', now())::date then s.week_minutes else 0 end, s.streak
  from profiles p join profile_stats s on s.user_id = p.id
  where (p.id = auth.uid() or (public.follows_accepted(auth.uid(), p.id) and p.share_stats))
  order by 5 desc, p.name
  limit 50
$$;

alter publication supabase_realtime add table public.song_shares;
