-- Row level security tests for the online features (scripts/test-db.sh).
-- Each block acts as a user (role + JWT claims) and checks what that user
-- can and can't do. Any failed check raises and stops the run.

create schema tests;
grant usage on schema tests to anon, authenticated, service_role;

-- Act as a user: a Discord account, an anonymous guest, or signed out.
create function tests.act(uid uuid, guest boolean default false) returns void language plpgsql as $$
begin
  if uid is null then
    perform set_config('role', 'anon', false);
    perform set_config('request.jwt.claim.sub', '', false);
    perform set_config('request.jwt.claims', '{"role":"anon"}', false);
  else
    perform set_config('role', 'authenticated', false);
    perform set_config('request.jwt.claim.sub', uid::text, false);
    perform set_config('request.jwt.claims', json_build_object('sub', uid, 'is_anonymous', guest)::text, false);
  end if;
end $$;

-- The statement must fail (optionally with a message containing `msg`).
create function tests.fails(stmt text, msg text default null) returns void language plpgsql as $$
begin
  begin
    execute stmt;
  exception when others then
    if msg is not null and position(msg in sqlerrm) = 0 then
      raise exception 'expected "%" from: % (got: %)', msg, stmt, sqlerrm;
    end if;
    return;
  end;
  raise exception 'expected failure: %', stmt;
end $$;

-- How many rows a query returns (as the current user).
create function tests.count(q text) returns bigint language plpgsql as $$
declare n bigint;
begin
  execute format('select count(*) from (%s) s', q) into n;
  return n;
end $$;

create function tests.check(ok boolean, what text) returns void language plpgsql as $$
begin
  if not coalesce(ok, false) then raise exception 'check failed: %', what; end if;
end $$;

grant execute on all functions in schema tests to anon, authenticated, service_role;

-- People: A (owner), B (friend), C (stranger), G (old guest account).
insert into auth.users (id, raw_user_meta_data, is_anonymous) values
  ('00000000-0000-0000-0000-00000000000a', '{"full_name":"Alex"}', false),
  ('00000000-0000-0000-0000-00000000000b', '{"full_name":"Bo"}', false),
  ('00000000-0000-0000-0000-00000000000c', '{"full_name":"Cy"}', false),
  ('00000000-0000-0000-0000-000000000099', '{}', true);

\set A '''00000000-0000-0000-0000-00000000000a'''
\set B '''00000000-0000-0000-0000-00000000000b'''
\set C '''00000000-0000-0000-0000-00000000000c'''
\set G '''00000000-0000-0000-0000-000000000099'''

-- C2: invite codes are only for the owner ----------------------------------------------
select tests.act(:A);
insert into playlists (id, title, visibility, collaborative, tracks)
  values ('11111111-1111-1111-1111-111111111111', 'Road trip', 'unlisted', true, '[{"key":"a::one","title":"One"}]');
select tests.check(length(playlist_invite('11111111-1111-1111-1111-111111111111')) = 32, 'owner gets a code');

select tests.act(:C);
select tests.check(tests.count($$select 1 from playlists where id = '11111111-1111-1111-1111-111111111111'$$) = 1, 'link viewers can open it');
select tests.fails($$select invite_code from playlists$$, 'invite_code');
select tests.fails($$select code from playlist_invites$$, 'permission denied');
select tests.fails($$select playlist_invite('11111111-1111-1111-1111-111111111111')$$, 'not allowed');
select tests.fails($$select rotate_playlist_invite('11111111-1111-1111-1111-111111111111')$$, 'not allowed');
select tests.fails($$select playlist_op('11111111-1111-1111-1111-111111111111', '{"remove":["a::one"]}')$$, 'not allowed');

select tests.act(null);
select tests.fails($$select code from playlist_invites$$, 'permission denied');
select tests.fails($$select playlist_invite('11111111-1111-1111-1111-111111111111')$$);

-- With the code (sent by the owner) B joins as an editor.
reset role;
select tests.act(:B);
do $$
declare c text;
begin
  perform set_config('role', 'postgres', false);
  select code into c from playlist_invites where playlist_id = '11111111-1111-1111-1111-111111111111';
  perform tests.act('00000000-0000-0000-0000-00000000000b');
  perform tests.check(join_playlist(c) = '11111111-1111-1111-1111-111111111111', 'B joins with the code');
end $$;
select tests.fails($$select join_playlist('nope')$$, 'invite not valid');

-- Rotating the code stops the old link.
reset role;
create temp table old_code as select code from playlist_invites;
grant select on old_code to authenticated;
select tests.act(:A);
select rotate_playlist_invite('11111111-1111-1111-1111-111111111111');
select tests.act(:C);
select tests.fails($$select join_playlist((select code from old_code))$$, 'invite not valid');

-- Collaborative edits: small change rows + one note for the owner ------------------------------
select tests.act(:B);
select playlist_op('11111111-1111-1111-1111-111111111111', '{"add":[{"key":"b::two","title":"Two"}],"at":0}');
select playlist_op('11111111-1111-1111-1111-111111111111', '{"move":["a::one"],"to":0}');
select tests.check((select tracks->0->>'key' from playlists where id = '11111111-1111-1111-1111-111111111111') = 'a::one', 'move applied');
select tests.check((select tracks->1->>'_by' from playlists where id = '11111111-1111-1111-1111-111111111111') = '00000000-0000-0000-0000-00000000000b', 'adds are stamped');
select tests.check(tests.count($$select 1 from playlist_changes where playlist_id = '11111111-1111-1111-1111-111111111111' and op ? 'add'$$) = 1, 'add published');
select tests.check((select op->>'to' from playlist_changes where op ? 'move') = '0', 'move published');
select tests.check((select max(version) from playlist_changes) = 3, 'versions follow the playlist');
select tests.act(:A);
select tests.check(tests.count($$select 1 from notifications where kind = 'playlist_activity'$$) = 1, 'owner told once, not per edit');
update playlists set title = 'Road trip 2026' where id = '11111111-1111-1111-1111-111111111111';
select tests.check(tests.count($$select 1 from playlist_changes where op->'meta'->>'title' = 'Road trip 2026'$$) = 1, 'renames reach followers');
select tests.check((select op->'meta' ? 'cover' from playlist_changes where op ? 'meta') = false, 'covers are not pushed in change rows');

-- Link previews follow the same visibility.
select tests.act(null);
select tests.check((select songs from playlist_preview('11111111-1111-1111-1111-111111111111')) = 2, 'preview of an unlisted playlist');
select tests.act(:A);
update playlists set visibility = 'private' where id = '11111111-1111-1111-1111-111111111111';
select tests.act(null);
select tests.check(tests.count($$select * from playlist_preview('11111111-1111-1111-1111-111111111111')$$) = 0, 'no preview of a private playlist');
select tests.act(:A);
update playlists set visibility = 'unlisted' where id = '11111111-1111-1111-1111-111111111111';

-- Old guest accounts can read but not write ---------------------------------------------------
select tests.act(:G, true);
select tests.check(tests.count($$select 1 from playlists$$) = 1, 'guests can open shared links');
select tests.fails($$insert into playlists (title) values ('spam')$$, 'row-level security');
select tests.fails($$insert into follows (followee) values ('00000000-0000-0000-0000-00000000000a')$$, 'row-level security');
select tests.fails($$insert into now_playing (track, playing) values ('{}', true)$$, 'row-level security');
select tests.fails($$select open_jam('guestroom1')$$, 'sign in first');

-- H3: follows need approval, activity is opt-in -----------------------------------------------------
select tests.act(:A);
update profiles set share_activity = true where id = auth.uid();
insert into now_playing (track, playing) values ('{"title":"Secret song"}', true);

select tests.act(:C);
insert into follows (followee) values (:A);
select tests.check((select status from follows where follower = auth.uid()) = 'pending', 'requests start pending');
select tests.check(tests.count('select 1 from now_playing') = 0, 'a pending follower sees nothing');
update follows set status = 'accepted' where follower = auth.uid();
select tests.check((select status from follows where follower = auth.uid()) = 'pending', 'followers cannot approve themselves');
select tests.fails($$insert into follows (followee, status) values ('00000000-0000-0000-0000-00000000000b', 'accepted')$$, 'row-level security');

select tests.act(:A);
select tests.check(tests.count($$select 1 from notifications where kind = 'follow_request'$$) = 1, 'request note');
select tests.fails($$update follows set follower = '00000000-0000-0000-0000-00000000000b' where followee = auth.uid()$$, 'permission denied');
update follows set status = 'accepted' where followee = auth.uid();

select tests.act(:C);
select tests.check(tests.count('select 1 from now_playing') = 1, 'approved followers see activity');
select tests.check(tests.count($$select 1 from notifications where kind = 'follow_accepted'$$) = 1, 'accepted note');

select tests.act(:A);
update profiles set share_activity = false where id = auth.uid();
select tests.act(:C);
select tests.check(tests.count('select 1 from now_playing') = 0, 'sharing off hides activity');

-- Wrapped: approved friends only, and only when shared.
select tests.act(:A);
insert into wrapped (year, data) values (2026, '{"minutes":1}');
select tests.act(:B);
select tests.check(tests.count('select 1 from wrapped') = 0, 'strangers do not see Wrapped');
select tests.act(:C);
select tests.check(tests.count('select 1 from wrapped') = 1, 'approved followers see Wrapped');
select tests.act(:A);
update profiles set share_wrapped = false where id = auth.uid();
select tests.act(:C);
select tests.check(tests.count('select 1 from wrapped') = 0, 'share_wrapped off hides it');

-- Removing a follower works from both sides.
select tests.act(:A);
delete from follows where follower = :C;
select tests.check(tests.count('select 1 from follows') = 0, 'follower removed');

-- H1: Listen Together ---------------------------------------------------------------------------
select tests.act(:A);
select tests.check(open_jam('roomabc123', false, 'Friday', false) = 'roomabc123', 'host opens a session');
update jams set state = '{"track":{"key":"a::one"},"playing":true,"position":12}' where code = 'roomabc123';
select tests.check((select state_at from jams where code = 'roomabc123') is not null, 'server stamps the state');

select tests.act(:B);
select tests.check(tests.count($$select 1 from jams where code = 'roomabc123'$$) = 0, 'outsiders cannot see a session');
select tests.fails($$insert into jam_events (code, kind, payload) values ('roomabc123', 'chat', '{"text":"hi"}')$$, 'row-level security');
select join_jam('roomabc123');
select tests.check(tests.count($$select 1 from jams where code = 'roomabc123'$$) = 1, 'members see it');
-- A guest's attempt to change what plays changes nothing.
update jams set state = '{"track":{"key":"evil"}}' where code = 'roomabc123';
select tests.check((select state->'track'->>'key' from jams where code = 'roomabc123') = 'a::one', 'only the host controls playback');
select tests.fails($$update jams set host = '00000000-0000-0000-0000-00000000000b' where code = 'roomabc123'$$, 'permission denied');
select tests.fails($$select jam_kick('roomabc123', '00000000-0000-0000-0000-00000000000a')$$, 'only the host');
select tests.fails($$select end_jam('roomabc123')$$, 'only the host');
insert into jam_events (code, kind, payload) values ('roomabc123', 'chat', '{"text":"hello","extra":"dropped"}');
select tests.check((select payload from jam_events where kind = 'chat') = '{"text": "hello"}', 'chat payload trimmed');
select tests.fails($$insert into jam_events (code, kind, payload) values ('roomabc123', 'chat', jsonb_build_object('text', repeat('x', 301)))$$, '1-300');
insert into jam_events (code, kind, payload, status) values ('roomabc123', 'suggest', '{"track":{"key":"b::two"}}', 'pending');
select tests.fails($$insert into jam_events (code, kind, payload, status) values ('roomabc123', 'suggest', '{}', 'approved')$$, 'row-level security');
update jam_events set status = 'approved' where kind = 'suggest';
select tests.check((select status from jam_events where kind = 'suggest') = 'pending', 'only the host answers suggestions');
do $$ begin
  for i in 1..18 loop insert into jam_events (code, kind, payload) values ('roomabc123', 'vote_skip', '{"key":"a::one"}'); end loop;
end $$;
select tests.fails($$insert into jam_events (code, kind, payload) values ('roomabc123', 'chat', '{"text":"spam"}')$$, 'slow down');

-- Realtime channel: members only.
select set_config('realtime.topic', 'jam:roomabc123', false);
insert into realtime.messages (topic, extension, event, payload) values ('jam:roomabc123', 'broadcast', 'react', '{}');
select tests.act(:C);
select tests.fails($$insert into realtime.messages (topic, extension, event, payload) values ('jam:roomabc123', 'broadcast', 'react', '{}')$$, 'row-level security');
select tests.check(tests.count('select 1 from realtime.messages') = 0, 'outsiders receive nothing');

-- Host answers suggestions, hands over, kicks.
select tests.act(:A);
update jam_events set status = 'approved' where kind = 'suggest';
select tests.check((select status from jam_events where kind = 'suggest') = 'approved', 'host approves');
select jam_handoff('roomabc123', :B);
select tests.act(:B);
update jams set state = '{"track":{"key":"b::two"}}' where code = 'roomabc123';
select tests.check((select state->'track'->>'key' from jams where code = 'roomabc123') = 'b::two', 'new host controls playback');
select jam_handoff('roomabc123', :A);
select tests.act(:A);
select jam_kick('roomabc123', :B);
select tests.act(:B);
select tests.check(tests.count($$select 1 from jam_events$$) = 0, 'kicked members see nothing');
select tests.fails($$select join_jam('roomabc123')$$, 'removed you');

-- Persistent room keeps its code.
select tests.act(:A);
select end_jam('roomabc123');
select tests.check(tests.count($$select 1 from jams where code = 'roomabc123'$$) = 0, 'ended sessions are gone');
select tests.check(open_jam('myroom0001', true, 'Alex''s room', false) = 'myroom0001', 'room created');
select end_jam('myroom0001');
select tests.check(open_jam('different01', true, '', false) = 'myroom0001', 'the room keeps its code');

-- Friends hear about a session when the host allows it.
reset role;
insert into follows (follower, followee, status) values (:C, :A, 'accepted');
select tests.act(:A);
select end_jam('myroom0001');
select open_jam('myroom0001', true, 'Party', true);
select tests.act(:C);
select tests.check(tests.count($$select 1 from notifications where kind = 'jam_started' and data->>'code' = 'myroom0001'$$) = 1, 'friends told');

-- Limits and the report rate limiter -------------------------------------------------------------
select tests.act(:B);
do $$ begin
  for i in 1..60 loop insert into playlists (title) values ('p' || i); end loop;
end $$;
select tests.fails($$insert into playlists (title) values ('one too many')$$, 'too many new playlists');
select tests.fails($$select report_allow('x', 1, 60)$$, 'permission denied');
reset role;
set role service_role;
select tests.check(report_allow('ip:1', 2, 60), 'first report');
select tests.check(report_allow('ip:1', 2, 60), 'second report');
select tests.check(not report_allow('ip:1', 2, 60), 'third is refused');
reset role;
