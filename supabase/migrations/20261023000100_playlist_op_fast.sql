-- playlist_op without quadratic work under the row lock.
--
-- Matching songs by key used `= any(array)` and array_position(), a scan of
-- the whole list per song: reordering 5 000 songs compared 25 million keys
-- while the playlist was locked. Keys now go into a jsonb object (sorted:
-- lookups are a binary search), so each op is O(n log n). Results are the
-- same, including what `= any()` and array_position() answer for null keys.
-- Ops are capped in size.

-- Keys as a jsonb object {key: true} (null keys left out).
create function public.key_set(keys text[]) returns jsonb
language sql immutable set search_path = public as $$
  select coalesce(jsonb_object_agg(k, true), '{}'::jsonb) from (select distinct k from unnest(keys) k where k is not null) s
$$;

-- `k = any(keys)` with s = key_set(keys), n = cardinality(keys) and
-- has_null = keys contains a null: true, false or null exactly like = any().
create function public.key_in(k text, s jsonb, n integer, has_null boolean) returns boolean
language sql immutable set search_path = public as $$
  select case when n = 0 then false when k is null then null when s ? k then true when has_null then null else false end
$$;

-- op: {"add": [tracks], "at": n?} | {"remove": [keys]} | {"move": [keys], "to": n}
--   | {"order": [keys]} | {"replace": [tracks]} (owner); optional "client": device id.
create or replace function public.playlist_op(pid uuid, op jsonb) returns jsonb
language plpgsql security definer set search_path = public as $$
declare
  cur jsonb;
  ver integer;
  result jsonb;
  keys text[];
  ks jsonb;
  kn integer;
  knull boolean;
  moving jsonb;
  rest jsonb;
  pos integer;
  published jsonb;
  client text := left(op->>'client', 64);
  pl record;
begin
  if not public.signed_in() or not public.can_edit(pid, auth.uid()) then raise exception 'not allowed'; end if;
  -- Bounded work under the lock (a whole library is ~10 000 songs).
  if octet_length(op::text) > 12000000
     or coalesce(jsonb_array_length(coalesce(op->'add', op->'remove', op->'move', op->'order', op->'replace')), 0) > 100000 then
    raise exception 'op too large';
  end if;
  select * into pl from playlists where id = pid for update;
  cur := pl.tracks;
  ver := pl.version;

  if op ? 'replace' then
    if pl.owner <> auth.uid() then raise exception 'only the owner can replace'; end if;
    result := op->'replace';
    published := jsonb_build_object('replace', true);
  elsif op ? 'add' then
    -- Keys already in the playlist, looked up once (not once per new song).
    keys := array(select c->>'key' from jsonb_array_elements(cur) c);
    ks := key_set(keys);
    kn := cardinality(keys);
    knull := array_position(keys, null) is not null;
    -- New songs (by key, each once) appended or inserted, stamped with who/when.
    select coalesce(jsonb_agg(t || jsonb_build_object('_by', auth.uid(), '_at', (extract(epoch from now()) * 1000)::bigint) order by n), '[]'::jsonb)
      into moving
      from (
        select distinct on (t->>'key') t, n from jsonb_array_elements(op->'add') with ordinality as x(t, n)
        where t->>'key' is not null and not key_in(t->>'key', ks, kn, knull)
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
    ks := key_set(keys);
    kn := cardinality(keys);
    knull := array_position(keys, null) is not null;
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into result
      from jsonb_array_elements(cur) with ordinality as x(e, n) where not key_in(e->>'key', ks, kn, knull);
    published := jsonb_build_object('remove', op->'remove');
  elsif op ? 'move' then
    keys := array(select jsonb_array_elements_text(op->'move'));
    ks := key_set(keys);
    kn := cardinality(keys);
    knull := array_position(keys, null) is not null;
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into moving from jsonb_array_elements(cur) with ordinality as x(e, n) where key_in(e->>'key', ks, kn, knull);
    select coalesce(jsonb_agg(e order by n), '[]'::jsonb) into rest from jsonb_array_elements(cur) with ordinality as x(e, n) where not key_in(e->>'key', ks, kn, knull);
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
    -- array_position(keys, key) as a lookup: each key's first position.
    select coalesce(jsonb_object_agg(k, i), '{}'::jsonb) into ks
      from (select k, min(i) as i from unnest(keys) with ordinality as u(k, i) where k is not null group by k) s;
    pos := array_position(keys, null);
    select coalesce(jsonb_agg(e order by coalesce(case when e->>'key' is null then pos else (ks->>(e->>'key'))::integer end, cardinality(keys) + n)), '[]'::jsonb) into result
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
