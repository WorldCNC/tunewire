-- Profile banners: the Discord profile banner (an image on Discord's CDN)
-- and accent color, read once at sign-in. Only Discord banner URLs are
-- accepted, so a profile can't point other people's apps at any server.
alter table public.profiles add column banner text
  check (banner is null or banner ~ '^https://cdn\.discordapp\.com/banners/[0-9]{5,25}/(a_)?[0-9a-f]{16,64}\.(png|gif|webp)(\?size=[0-9]{2,4})?$');
alter table public.profiles add column accent_color integer
  check (accent_color is null or accent_color between 0 and 16777215);
