-- Your own picture and banner (scripts/test-db.sh): only your own folder in
-- the profile-media bucket, only allowed addresses on the profile.
-- Uses tests.* helpers from test_policies.sql (runs after it: files go in name order).

\set A '''00000000-0000-0000-0000-00000000000a'''
\set B '''00000000-0000-0000-0000-00000000000b'''

-- Uploads: your folder only, image names only, no guests.
select tests.act(:A);
insert into storage.objects (bucket_id, name) values ('profile-media', '00000000-0000-0000-0000-00000000000a/0123456789abcdef0123.gif');
select tests.fails($$insert into storage.objects (bucket_id, name) values ('profile-media', '00000000-0000-0000-0000-00000000000b/0123456789abcdef0123.gif')$$, 'row-level security');
select tests.fails($$insert into storage.objects (bucket_id, name) values ('profile-media', '00000000-0000-0000-0000-00000000000a/../x.html')$$, 'row-level security');
select tests.fails($$insert into storage.objects (bucket_id, name) values ('profile-media', '00000000-0000-0000-0000-00000000000a/0123456789abcdef0123.svg')$$, 'row-level security');
select tests.act(:B, true);
select tests.fails($$insert into storage.objects (bucket_id, name) values ('profile-media', '00000000-0000-0000-0000-00000000000b/0123456789abcdef0123.png')$$, 'row-level security');

-- Removing: only your own files.
select tests.act(:B);
delete from storage.objects where name like '00000000-0000-0000-0000-00000000000a/%';
reset role;
select tests.check((select count(*) from storage.objects where name like '00000000-0000-0000-0000-00000000000a/%') = 1, 'someone else can''t remove your files');
select tests.act(:A);
delete from storage.objects where name like '00000000-0000-0000-0000-00000000000a/%';
reset role;
select tests.check((select count(*) from storage.objects where name like '00000000-0000-0000-0000-00000000000a/%') = 0, 'you can remove your own');

-- The profile: your own uploads or Discord's images, nothing else.
select tests.act(:A);
update profiles set avatar = 'media:00000000-0000-0000-0000-00000000000a/0123456789abcdef0123.webp', banner = 'media:00000000-0000-0000-0000-00000000000a/0123456789abcdef0123.gif' where id = :A;
update profiles set avatar = 'https://cdn.discordapp.com/avatars/80351110224678912/a_06c16474723fe537c283b8efa61a30c8.gif?size=256' where id = :A;
update profiles set banner = 'https://cdn.discordapp.com/banners/80351110224678912/a_06c16474723fe537c283b8efa61a30c8.gif?size=1024' where id = :A;
update profiles set avatar = null, banner = null where id = :A;
select tests.fails($$update profiles set avatar = 'media:00000000-0000-0000-0000-00000000000b/0123456789abcdef0123.png' where id = '00000000-0000-0000-0000-00000000000a'$$, 'profiles_avatar_source');
select tests.fails($$update profiles set banner = 'media:00000000-0000-0000-0000-00000000000b/0123456789abcdef0123.png' where id = '00000000-0000-0000-0000-00000000000a'$$, 'profiles_banner_check');
select tests.fails($$update profiles set avatar = 'https://evil.example/track.png' where id = '00000000-0000-0000-0000-00000000000a'$$, 'profiles_avatar_source');
select tests.fails($$update profiles set banner = 'https://evil.example/b.gif' where id = '00000000-0000-0000-0000-00000000000a'$$, 'profiles_banner_check');
update profiles set discord_imported = true where id = :A;
select tests.check((select discord_imported from profiles where id = :A), 'the one-time Discord import can be marked');
reset role;
