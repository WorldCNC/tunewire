-- Library sync across devices (scripts/test-db.sh): private copies, Liked
-- Songs, tombstones, device ids on change rows, the order op, limits.
-- Uses tests.* helpers from test_policies.sql (runs after it: files go in name order).

\set A '''00000000-0000-0000-0000-00000000000a'''
\set B '''00000000-0000-0000-0000-00000000000b'''
\set C '''00000000-0000-0000-0000-00000000000c'''

select tests.act(:A);
insert into playlists (id, title, visibility, local_id, tracks) values
  ('22222222-2222-2222-2222-222222222222', 'Mine', 'private', 'pl1', '[{"key":"k1"},{"key":"k2"},{"key":"k3"}]');
select tests.fails($$insert into playlists (title, visibility, local_id) values ('Again', 'private', 'pl1')$$, 'duplicate key');
insert into playlists (id, title, kind, visibility) values ('33333333-3333-3333-3333-333333333333', 'Liked Songs', 'liked', 'private');
select tests.fails($$insert into playlists (title, kind, visibility) values ('Liked 2', 'liked', 'private')$$, 'duplicate key');
select tests.fails($$update playlists set visibility = 'unlisted' where id = '33333333-3333-3333-3333-333333333333'$$, 'liked_is_private');
select tests.fails($$update playlists set kind = 'liked' where id = '22222222-2222-2222-2222-222222222222'$$, 'can''t change');

-- Ops carry the device; change rows know the owner.
select playlist_op('22222222-2222-2222-2222-222222222222', '{"add":[{"key":"k4"},{"key":"k4"},{"key":"k1"}],"at":1,"client":"dev-1"}');
select tests.check((select jsonb_agg(t->>'key') from playlists p, jsonb_array_elements(p.tracks) t where p.id = '22222222-2222-2222-2222-222222222222') = '["k1","k4","k2","k3"]', 'add: inserted once, existing skipped');
select tests.check((select client from playlist_changes where playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = 'dev-1', 'change row has the device');
select tests.check((select owner from playlist_changes where playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = :A, 'change row has the owner');
select tests.check((select op->'add'->0 ? 'client' from playlist_changes where playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = false, 'device id is not inside the songs');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"order":["k3","k1"]}');
select tests.check((select jsonb_agg(t->>'key') from playlists p, jsonb_array_elements(p.tracks) t where p.id = '22222222-2222-2222-2222-222222222222') = '["k3","k1","k4","k2"]', 'order: listed first, the rest keep their order');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"move":["k2"],"to":1}');
select tests.check((select jsonb_agg(t->>'key') from playlists p, jsonb_array_elements(p.tracks) t where p.id = '22222222-2222-2222-2222-222222222222') = '["k3","k2","k1","k4"]', 'move');
select tests.check((select version from playlists where id = '22222222-2222-2222-2222-222222222222') = 4, 'every op is one version');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"order":["k4","k4","nope"]}');
select tests.check((select jsonb_agg(t->>'key') from playlists p, jsonb_array_elements(p.tracks) t where p.id = '22222222-2222-2222-2222-222222222222') = '["k4","k3","k2","k1"]', 'order: repeated and unknown keys');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"remove":["k3","nope"]}');
select tests.check((select jsonb_agg(t->>'key') from playlists p, jsonb_array_elements(p.tracks) t where p.id = '22222222-2222-2222-2222-222222222222') = '["k4","k2","k1"]', 'remove');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"add":[{"key":"k3"}],"at":0}');
select playlist_op('22222222-2222-2222-2222-222222222222', '{"order":["k1","k2","k3","k4"]}');
select tests.fails($$select playlist_op('22222222-2222-2222-2222-222222222222', jsonb_build_object('remove', (select jsonb_agg(g::text) from generate_series(1, 100001) g)))$$, 'op too large');
select tests.check((select version from playlists where id = '22222222-2222-2222-2222-222222222222') = 8, 'a refused op changes nothing');

-- Meta changes say which device and what changed.
update playlists set title = 'Mine!', folder = 'Gym', pinned = true, last_client = 'dev-2' where id = '22222222-2222-2222-2222-222222222222';
select tests.check((select client from playlist_changes where op ? 'meta' and playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = 'dev-2', 'meta change has the device');
select tests.check((select op->'meta'->>'folder' from playlist_changes where op ? 'meta' and playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = 'Gym', 'folder in meta');
select tests.check((select cover_md5 from playlists where id = '22222222-2222-2222-2222-222222222222') = md5(''), 'cover hash');

-- Private copies are invisible to everyone else.
select tests.act(:C);
select tests.check(tests.count($$select 1 from playlists where id in ('22222222-2222-2222-2222-222222222222', '33333333-3333-3333-3333-333333333333')$$) = 0, 'private rows hidden');
select tests.check(tests.count($$select 1 from playlist_changes where playlist_id = '22222222-2222-2222-2222-222222222222'$$) = 0, 'their changes hidden');

-- Tombstones: owner still sees them, others lose access, no more edits.
select tests.act(:A);
update playlists set visibility = 'unlisted' where id = '22222222-2222-2222-2222-222222222222';
select tests.act(:C);
select tests.check(tests.count($$select 1 from playlists where id = '22222222-2222-2222-2222-222222222222'$$) = 1, 'shared: visible');
select tests.act(:A);
update playlists set deleted_at = now(), last_client = 'dev-2' where id = '22222222-2222-2222-2222-222222222222';
select tests.check((select op->'meta'->>'deleted' from playlist_changes where op ? 'meta' and playlist_id = '22222222-2222-2222-2222-222222222222' order by id desc limit 1) = 'true', 'delete is announced');
select tests.check(tests.count($$select 1 from playlists where id = '22222222-2222-2222-2222-222222222222'$$) = 1, 'owner still sees the tombstone');
select tests.fails($$select playlist_op('22222222-2222-2222-2222-222222222222', '{"remove":["k1"]}')$$, 'not allowed');
-- A new copy of the same local playlist is allowed once the old one is deleted.
insert into playlists (id, title, visibility, local_id) values ('44444444-4444-4444-4444-444444444444', 'Mine', 'private', 'pl1');
delete from playlists where id = '44444444-4444-4444-4444-444444444444';
select tests.act(:C);
select tests.check(tests.count($$select 1 from playlists where id = '22222222-2222-2222-2222-222222222222'$$) = 0, 'deleted: gone for others');
select tests.fails($$insert into playlist_members (playlist_id, role) values ('22222222-2222-2222-2222-222222222222', 'follower')$$, 'row-level security');
-- Undelete brings it back.
select tests.act(:A);
update playlists set deleted_at = null where id = '22222222-2222-2222-2222-222222222222';
select tests.act(:C);
select tests.check(tests.count($$select 1 from playlists where id = '22222222-2222-2222-2222-222222222222'$$) = 1, 'restored');

-- Limits: private copies don't use the shared allowance.
select tests.act(:C);
do $$ begin
  for i in 1..100 loop insert into playlists (title, visibility, local_id) values ('p' || i, 'private', 'l' || i); end loop;
end $$;
select tests.check(tests.count($$select 1 from playlists where owner = auth.uid()$$) = 100, 'a big library uploads at once');
insert into playlists (title, visibility) values ('shared one', 'unlisted');
select tests.act(:A);
select tests.check(tests.count($$select 1 from playlist_changes where playlist_id = '33333333-3333-3333-3333-333333333333' and op->'meta'->>'created' = 'true' and owner = auth.uid()$$) = 1, 'creation is announced to your devices');

-- Hardening ---------------------------------------------------------------------------------------
select tests.act(:A);
select tests.fails($$update profiles set created_at = '2020-01-01' where id = auth.uid()$$, 'permission denied');
update profiles set bio = 'hi', pronouns = 'they/them', share_stats = true where id = auth.uid();
select tests.check((select bio from profiles where id = auth.uid()) = 'hi', 'own bio still editable');
-- Stats: no faster than time passes.
insert into profile_stats (week_start, week_minutes) values (current_date, 10000)
  on conflict (user_id) do update set week_start = excluded.week_start, week_minutes = excluded.week_minutes;
select tests.check((select week_minutes from profile_stats where user_id = auth.uid()) <= ceil(extract(epoch from now() - (current_date::timestamptz - interval '14 hours')) / 60) + 1, 'week minutes capped by time passed (since the earliest local midnight)');
select tests.check((select week_minutes from profile_stats where user_id = auth.uid()) >= 14 * 60, 'UTC+14 users keep minutes from before UTC midnight');
update profile_stats set week_minutes = week_minutes + 5000 where user_id = auth.uid();
select tests.check((select week_minutes from profile_stats where user_id = auth.uid()) <= ceil(extract(epoch from now() - (current_date::timestamptz - interval '14 hours')) / 60) + 6, 'no jumps between updates');
-- Search: signed in, 2+ characters, at most 20, and % is just a character.
select tests.check(tests.count($$select * from search_profiles('Bo')$$) = 1, 'finds Bo');
select tests.check(tests.count($$select * from search_profiles('%')$$) = 0, '% is not a wildcard');
select tests.check(tests.count($$select * from search_profiles('B')$$) = 0, 'one letter: nothing');
select tests.act(null);
select tests.fails($$select * from search_profiles('Bo')$$);
-- Any change moves updated_at.
select tests.act(:A);
create temp table before_touch as select updated_at from playlists where id = '22222222-2222-2222-2222-222222222222';
select pg_sleep(0.01);
update playlists set pinned = not pinned where id = '22222222-2222-2222-2222-222222222222';
select tests.check((select updated_at from playlists where id = '22222222-2222-2222-2222-222222222222') > (select updated_at from before_touch), 'updated_at moves');
