-- Signed out, profiles show only their public face.
--
--  * Before, anyone with the public anon key could read every column of
--    every profile (profiles?select=*), including bios and sharing choices.
--    Now signed-out callers can read just the columns a share page or a
--    shared playlist shows: name, picture, banner, accent color and Discord
--    decoration (playlists?select=...,profiles(name) and
--    profiles?select=name,avatar keep working for v2.0.1 and the website).
--    Signed-in people read profiles as before (friends lists, playlist
--    owners and Listen Together names embed them).
--  * Link previews (share Edge Function, signed out) get the few public
--    fields of one profile from profile_preview(uid), and a playlist's owner
--    name through playlist_preview() as before.
--  * Policies that look at someone's sharing choices read them through a
--    helper, so they work (and stay empty) for signed-out callers too.

revoke select on public.profiles from anon;
grant select (id, name, avatar, banner, accent_color, decoration) on public.profiles to anon;
drop policy "profiles are public" on public.profiles;
create policy "profiles for signed-in people" on public.profiles for select to authenticated using (true);
create policy "public profile fields signed out" on public.profiles for select to anon using (true);

-- One profile's public face: what a link preview shows.
create function public.profile_preview(uid uuid)
returns table (id uuid, name text, avatar text, banner text)
language sql stable security definer set search_path = public as $$
  select p.id, p.name, p.avatar, p.banner from profiles p where p.id = uid
$$;
grant execute on function public.profile_preview(uuid) to anon, authenticated;

-- Whether `uid` shares their activity / Wrapped / stats (for policies).
create function public.profile_shares(uid uuid, what text) returns boolean
language sql stable security definer set search_path = public as $$
  select coalesce((
    select case what when 'activity' then p.share_activity when 'wrapped' then p.share_wrapped when 'stats' then p.share_stats else false end
    from profiles p where p.id = uid), false)
$$;

drop policy "followers see activity" on public.now_playing;
create policy "followers see activity" on public.now_playing for select using (
  user_id = auth.uid() or (public.follows_accepted(auth.uid(), user_id) and public.profile_shares(user_id, 'activity'))
);

drop policy "approved friends see wrapped" on public.wrapped;
create policy "approved friends see wrapped" on public.wrapped for select using (
  user_id = auth.uid() or (
    (public.follows_accepted(auth.uid(), user_id) or public.follows_accepted(user_id, auth.uid()))
    and public.profile_shares(user_id, 'wrapped')
  )
);

drop policy "shared stats for approved followers" on public.profile_stats;
create policy "shared stats for approved followers" on public.profile_stats for select using (
  user_id = auth.uid() or (public.follows_accepted(auth.uid(), user_id) and public.profile_shares(user_id, 'stats'))
);

-- Link previews: the owner's name without reading profiles as the caller
-- (the playlist itself still goes through the caller's row level security).
create or replace function public.playlist_preview(pid uuid)
returns table (title text, description text, owner_name text, songs integer, thumbnail text, has_cover boolean)
language sql stable security invoker set search_path = public as $$
  select p.title, p.description, (select pr.name from public.profile_preview(p.owner) pr), jsonb_array_length(p.tracks), p.tracks->0->>'thumbnail', p.cover is not null
  from playlists p where p.id = pid
$$;
