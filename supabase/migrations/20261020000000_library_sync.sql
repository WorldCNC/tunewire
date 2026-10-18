-- Tunewire online features: your library on all your devices.
--
--  * Every playlist can have a private online copy (visibility 'private',
--    nobody else can open it), plus one "Liked Songs" row per person
--    (kind 'liked'). Each device keeps its own copy and syncs it.
--  * Deleting is a tombstone (deleted_at), so your other devices learn about
--    it; tombstones are purged after 30 days. Deleted playlists disappear for
--    everyone else at once.
--  * Change rows say which device made them (client) and whose playlist it is
--    (owner): a device skips only its own changes, and listens to all of your
--    playlists on one realtime channel (owner=eq.<you>).
--  * New op "order" (the whole order at once); "add" no longer adds the same
--    song twice from one request.
--  * Folder and pin sync too; cover_md5 tells devices the cover changed
--    without downloading it.
--  * Limits: private copies don't count toward the 300 shared playlists, and
--    a big library can be uploaded at once (2000 rows, 500 an hour).

alter table public.playlists
  add column kind text not null default 'playlist' check (kind in ('playlist', 'liked')),
  add column local_id text check (local_id is null or char_length(local_id) <= 64),
  add column folder text check (folder is null or char_length(folder) <= 60),
  add column pinned boolean not null default false,
  add column deleted_at timestamptz,
  -- Device that made the last change to the row itself (rename, cover, …).
  add column last_client text check (last_client is null or char_length(last_client) <= 64),
  add column cover_md5 text generated always as (md5(coalesce(cover, ''))) stored;

-- One online copy per local playlist and device library, one Liked Songs.
create unique index playlists_owner_local on public.playlists (owner, local_id) where local_id is not null and deleted_at is null;
create unique index playlists_owner_liked on public.playlists (owner) where kind = 'liked' and deleted_at is null;
-- Liked Songs is always private and never collaborative.
alter table public.playlists add constraint liked_is_private check (kind <> 'liked' or (visibility = 'private' and not collaborative));

alter table public.playlist_changes
  add column owner uuid references public.profiles on delete cascade,
  add column client text check (client is null or char_length(client) <= 64);
update public.playlist_changes c set owner = p.owner from public.playlists p where p.id = c.playlist_id;
create index playlist_changes_owner on public.playlist_changes (owner, id);

-- Deleted playlists: only the owner still sees them (to learn about the delete).
create or replace function public.can_read_playlist(pid uuid, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (select 1 from playlists p where p.id = pid and (
    p.owner = uid
    or (p.deleted_at is null and (p.visibility in ('public', 'unlisted') or public.is_member(pid, uid)))))
$$;

drop policy "read shared playlists" on public.playlists;
create policy "read shared playlists" on public.playlists for select
  using (owner = auth.uid() or (deleted_at is null and (visibility in ('public', 'unlisted') or public.is_member(id, auth.uid()))));

drop policy "follow readable playlists" on public.playlist_members;
create policy "follow readable playlists" on public.playlist_members for insert
  with check (user_id = auth.uid() and role = 'follower' and public.signed_in()
    and exists (select 1 from public.playlists p where p.id = playlist_id and p.visibility in ('public', 'unlisted') and p.deleted_at is null));

create or replace function public.can_edit(pid uuid, uid uuid) returns boolean
language sql security definer stable set search_path = public as $$
  select exists (
    select 1 from playlists p where p.id = pid and p.deleted_at is null and (
      p.owner = uid
      or (p.collaborative and exists (select 1 from playlist_members m where m.playlist_id = pid and m.user_id = uid and m.role = 'editor'))
    )
  )
$$;

create or replace function public.join_playlist(code text) returns uuid
language plpgsql security definer set search_path = public as $$
declare pid uuid;
begin
  if not public.signed_in() then raise exception 'sign in first'; end if;
  select i.playlist_id into pid from playlist_invites i join playlists p on p.id = i.playlist_id
    where i.code = join_playlist.code and p.collaborative and p.deleted_at is null;
  if pid is null then raise exception 'invite not valid'; end if;
  insert into playlist_members (playlist_id, user_id, role) values (pid, auth.uid(), 'editor')
    on conflict (playlist_id, user_id) do update set role = 'editor';
  return pid;
end $$;

-- Limits ---------------------------------------------------------------------------------
create or replace function public.limit_playlists() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  -- Old tombstones go first (nothing else purges them).
  delete from playlists where owner = new.owner and deleted_at < now() - interval '30 days';
  if (select count(*) from playlists where owner = new.owner and deleted_at is null) >= 2000 then
    raise exception 'too many playlists (2000 max)';
  end if;
  if (select count(*) from playlists where owner = new.owner and created_at > now() - interval '1 hour') >= 500 then
    raise exception 'too many new playlists, try again later';
  end if;
  if new.visibility <> 'private' then
    if (select count(*) from playlists where owner = new.owner and visibility <> 'private' and deleted_at is null) >= 300 then
      raise exception 'too many shared playlists (300 max)';
    end if;
    if (select count(*) from playlists where owner = new.owner and visibility <> 'private' and created_at > now() - interval '1 hour') >= 60 then
      raise exception 'too many new playlists, try again later';
    end if;
  end if;
  return new;
end $$;

-- Making a private copy shared counts toward the shared limit too.
create function public.limit_shared_playlists() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if new.visibility <> 'private' and old.visibility = 'private'
     and (select count(*) from playlists where owner = new.owner and visibility <> 'private' and deleted_at is null) >= 300 then
    raise exception 'too many shared playlists (300 max)';
  end if;
  if new.kind <> old.kind or new.local_id is distinct from old.local_id and old.local_id is not null then
    raise exception 'kind and local_id can''t change';
  end if;
  return new;
end $$;
create trigger limit_shared_playlists before update on public.playlists for each row execute function public.limit_shared_playlists();

-- Change rows -------------------------------------------------------------------------------
-- Meta changes (title, description, cover, visibility, folder, pin, delete).
create or replace function public.playlist_meta_changed() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if new.title is distinct from old.title or new.description is distinct from old.description
     or new.cover is distinct from old.cover or new.visibility is distinct from old.visibility
     or new.collaborative is distinct from old.collaborative or new.folder is distinct from old.folder
     or new.pinned is distinct from old.pinned or new.deleted_at is distinct from old.deleted_at then
    insert into playlist_changes (playlist_id, version, op, by, owner, client) values (new.id, new.version, jsonb_build_object('meta', jsonb_build_object(
      'title', new.title, 'description', new.description, 'visibility', new.visibility, 'collaborative', new.collaborative,
      'folder', new.folder, 'pinned', new.pinned, 'deleted', new.deleted_at is not null,
      'cover_changed', new.cover is distinct from old.cover)), auth.uid(), new.owner, new.last_client);
  end if;
  return new;
end $$;

-- op: {"add": [tracks], "at": n?} | {"remove": [keys]} | {"move": [keys], "to": n}
--   | {"order": [keys]} | {"replace": [tracks]} (owner); optional "client": device id.
create or replace function public.playlist_op(pid uuid, op jsonb) returns jsonb
language plpgsql security definer set search_path = public as $$
declare
  cur jsonb;
  ver integer;
  result jsonb;
  keys text[];
  have text[];
  moving jsonb;
  rest jsonb;
  pos integer;
  published jsonb;
  client text := left(op->>'client', 64);
  pl record;
begin
  if not public.signed_in() or not public.can_edit(pid, auth.uid()) then raise exception 'not allowed'; end if;
  select * into pl from playlists where id = pid for update;
  cur := pl.tracks;
  ver := pl.version;
  -- Keys already in the playlist, looked up once (not once per new song).
  have := array(select c->>'key' from jsonb_array_elements(cur) c);

  if op ? 'replace' then
    if pl.owner <> auth.uid() then raise exception 'only the owner can replace'; end if;
    result := op->'replace';
    published := jsonb_build_object('replace', true);
  elsif op ? 'add' then
    -- New songs (by key, each once) appended or inserted, stamped with who/when.
    select coalesce(jsonb_agg(t || jsonb_build_object('_by', auth.uid(), '_at', (extract(epoch from now()) * 1000)::bigint) order by n), '[]'::jsonb)
      into moving
      from (
        select distinct on (t->>'key') t, n from jsonb_array_elements(op->'add') with ordinality as x(t, n)
        where t->>'key' is not null and not (t->>'key' = any(have))
        order by t->>'key', n
      ) s;
    pos := least(greatest(coalesce((op->>'at')::integer, jsonb_array_length(cur)), 0), jsonb_array_length(cur));
    select coalesce(jsonb_agg(e order by ord), '[]'::jsonb) into result from (
      select e, n::numeric as ord from jsonb_array_elements(cur) with ordinality as x(e, n) where n <= pos
      union all
      select e, pos + n::numeric / (jsonb_array_length(moving) + 1) from jsonb_array_elements(moving) with ordinality as y(e, n)
      union all
      select e, n::numeric + 1 from jsonb_array_elements(cur) with ordinality as x(e, n) where n > pos
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
      select e, n::numeric as ord from jsonb_array_elements(rest) with ordinality as x(e, n) where n <= pos
      union all
      select e, pos + n::numeric / (jsonb_array_length(moving) + 1) from jsonb_array_elements(moving) with ordinality as y(e, n)
      union all
      select e, n::numeric + 1 from jsonb_array_elements(rest) with ordinality as x(e, n) where n > pos
    ) s;
    published := jsonb_build_object('move', op->'move', 'to', pos);
  elsif op ? 'order' then
    -- Listed songs in the given order; anything not listed (added meanwhile
    -- by someone else) keeps its relative order after them.
    keys := array(select jsonb_array_elements_text(op->'order'));
    select coalesce(jsonb_agg(e order by coalesce(array_position(keys, e->>'key'), cardinality(keys) + n)), '[]'::jsonb) into result
      from jsonb_array_elements(cur) with ordinality as x(e, n);
    published := jsonb_build_object('order', op->'order');
  else
    raise exception 'unknown op';
  end if;

  update playlists set tracks = result, version = ver + 1, updated_at = now() where id = pid;
  insert into playlist_changes (playlist_id, version, op, by, owner, client) values (pid, ver + 1, published, auth.uid(), pl.owner, client);
  delete from playlist_changes where playlist_id = pid and created_at < now() - interval '7 days';

  -- Tell the owner and the other editors (at most once an hour per person).
  if pl.collaborative and not (op ? 'replace') then
    perform public.notify(r.uid, 'playlist_activity', auth.uid(), jsonb_build_object('playlist', pid, 'title', pl.title))
      from (select pl.owner as uid union select user_id from playlist_members where playlist_id = pid and role = 'editor') r;
  end if;
  return jsonb_build_object('version', ver + 1, 'count', jsonb_array_length(result));
end $$;

-- A new playlist (or Liked Songs) tells your other devices right away.
create function public.playlist_created() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  insert into playlist_changes (playlist_id, version, op, by, owner, client) values (new.id, new.version,
    jsonb_build_object('meta', jsonb_build_object('created', true, 'kind', new.kind)), auth.uid(), new.owner, new.last_client);
  return new;
end $$;
create trigger playlist_created after insert on public.playlists for each row execute function public.playlist_created();

-- Whole libraries sync now (Liked Songs can be thousands of songs): 12 MB of
-- song info per playlist instead of 3 MB.
alter table public.playlists drop constraint playlists_tracks_check;
alter table public.playlists add constraint playlists_tracks_check
  check (jsonb_typeof(tracks) = 'array' and octet_length(tracks::text) < 12000000);
