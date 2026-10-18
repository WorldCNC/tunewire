-- Your own profile picture and banner (GIFs too).
--
--  * Pictures and banners can be uploaded to the public "profile-media"
--    bucket, each person only into their own folder (<user id>/...), at most
--    8 MB, PNG/JPEG/WebP/GIF only. The profile stores "media:<user id>/<file>",
--    which the app turns into the bucket's address; it must be your own folder.
--  * Otherwise only Discord's own image addresses are accepted, as before for
--    banners: a profile can't point other people's apps at any server.
--    (Checked for new writes; older rows are left as they are.)
--  * The Discord picture comes with a new account and the Discord banner,
--    accent color and avatar decoration are copied once (discord_imported);
--    after that only you change them.

insert into storage.buckets (id, name, public, file_size_limit, allowed_mime_types)
values ('profile-media', 'profile-media', true, 8388608, array['image/png', 'image/jpeg', 'image/webp', 'image/gif'])
on conflict (id) do update set public = excluded.public, file_size_limit = excluded.file_size_limit, allowed_mime_types = excluded.allowed_mime_types;

create policy "profile media: add to own folder" on storage.objects for insert to authenticated
  with check (bucket_id = 'profile-media' and public.signed_in() and (storage.foldername(name))[1] = auth.uid()::text
    and name ~ '^[0-9a-f-]{36}/[0-9a-f]{16,64}\.(png|jpg|webp|gif)$');
-- (Anyone can view the files through the public address; listing and
-- removing go through these rows.)
create policy "profile media: see own folder" on storage.objects for select to authenticated
  using (bucket_id = 'profile-media' and (storage.foldername(name))[1] = auth.uid()::text);
create policy "profile media: remove from own folder" on storage.objects for delete to authenticated
  using (bucket_id = 'profile-media' and (storage.foldername(name))[1] = auth.uid()::text);

-- Profile addresses ---------------------------------------------------------------------------
alter table public.profiles drop constraint if exists profiles_banner_check;
alter table public.profiles add constraint profiles_banner_check check (
  banner is null
  or banner ~ '^https://cdn\.discordapp\.com/banners/[0-9]{5,25}/(a_)?[0-9a-f]{16,64}\.(png|gif|webp)(\?size=[0-9]{2,4})?$'
  or banner ~ ('^media:' || id::text || '/[0-9a-f]{16,64}\.(png|jpg|webp|gif)$'));
alter table public.profiles add constraint profiles_avatar_source check (
  avatar is null
  or avatar ~ '^https://cdn\.discordapp\.com/(avatars/[0-9]{5,25}/(a_)?[0-9a-f]{16,64}|embed/avatars/[0-9]{1,2})\.(png|gif|webp|jpg)(\?size=[0-9]{2,4})?$'
  or avatar ~ ('^media:' || id::text || '/[0-9a-f]{16,64}\.(png|jpg|webp|gif)$')) not valid;

alter table public.profiles add column discord_imported boolean not null default false;
grant update (discord_imported) on public.profiles to authenticated;
