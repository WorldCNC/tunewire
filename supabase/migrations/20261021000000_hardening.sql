-- Hardening after an audit.
--
--  * Profiles: you can change your own name, picture, look, bio and privacy
--    choices, nothing else (before, any column: e.g. created_at, which the
--    "early" badge is based on).
--  * Shared listening stats can't grow faster than time passes. (Streaks
--    aren't capped: they come from the local play log, which can be older
--    than the account.)
--  * Finding people goes through search_profiles(): at least 2 characters,
--    at most 20 results, signed in only.
--  * playlists.updated_at follows every change (renames, deletes, …), so a
--    device can ask for "what changed since" instead of everything.

-- Profiles: only these columns can be changed by their owner.
revoke update on public.profiles from anon, authenticated;
grant update (name, avatar, banner, accent_color, decoration, bio, pronouns, share_activity, share_stats, share_wrapped)
  on public.profiles to authenticated;

-- Listening stats --------------------------------------------------------------------------
create function public.sane_profile_stats() returns trigger
language plpgsql security definer set search_path = public as $$
declare
  since_update double precision;
begin
  -- This week's minutes: no more than have passed since the week started.
  -- week_start is the user's local date; their midnight can be up to 14 hours
  -- before UTC midnight (UTC+14).
  if new.week_start is not null then
    new.week_minutes := least(new.week_minutes, greatest(0, ceil(extract(epoch from now() - (new.week_start::timestamptz - interval '14 hours')) / 60)::integer));
  end if;
  -- And between two updates in the same week, no faster than real time.
  if tg_op = 'UPDATE' and new.week_start is not distinct from old.week_start then
    since_update := extract(epoch from now() - old.updated_at) / 60;
    new.week_minutes := least(new.week_minutes, old.week_minutes + ceil(since_update)::integer + 5);
    new.month_minutes := least(new.month_minutes, old.month_minutes + ceil(since_update)::integer + 5);
  end if;
  new.updated_at := now();
  return new;
end $$;
create trigger sane_profile_stats before insert or update on public.profile_stats
  for each row execute function public.sane_profile_stats();

-- Finding people ---------------------------------------------------------------------------
create function public.search_profiles(q text) returns setof public.profiles
language plpgsql stable security invoker set search_path = public as $$
declare
  pattern text := '%' || replace(replace(replace(trim(q), '\', '\\'), '%', '\%'), '_', '\_') || '%';
begin
  if not public.signed_in() then raise exception 'sign in first'; end if;
  if char_length(trim(q)) < 2 then return; end if;
  return query select * from profiles where name ilike pattern order by char_length(name), name limit 20;
end $$;
revoke execute on function public.search_profiles(text) from anon;

-- Every change to a playlist row moves updated_at ----------------------------------------------
create function public.touch_playlist() returns trigger
language plpgsql set search_path = public as $$
begin
  new.updated_at := now();
  return new;
end $$;
create trigger touch_playlist before update on public.playlists for each row execute function public.touch_playlist();
create index playlists_owner_updated on public.playlists (owner, updated_at);
