-- Minimal stand-ins for what a Supabase project provides, so the migrations
-- and their row level security can be tested on plain Postgres
-- (scripts/test-db.sh). Never applied to a real project.
create role anon nologin;
create role authenticated nologin;
create role service_role nologin bypassrls;
grant usage on schema public to anon, authenticated, service_role;
alter default privileges in schema public grant all on tables to anon, authenticated, service_role;
alter default privileges in schema public grant all on sequences to anon, authenticated, service_role;
alter default privileges in schema public grant all on functions to anon, authenticated, service_role;

create schema auth;
grant usage on schema auth to anon, authenticated, service_role;
create table auth.users (id uuid primary key, raw_user_meta_data jsonb not null default '{}', is_anonymous boolean not null default false);
create function auth.uid() returns uuid language sql stable as $$ select nullif(current_setting('request.jwt.claim.sub', true), '')::uuid $$;
create function auth.jwt() returns jsonb language sql stable as $$ select coalesce(nullif(current_setting('request.jwt.claims', true), ''), '{}')::jsonb $$;

create schema realtime;
grant usage on schema realtime to anon, authenticated;
create table realtime.messages (id bigserial primary key, topic text, extension text, event text, payload jsonb, private boolean default true);
alter table realtime.messages enable row level security;
grant select, insert on realtime.messages to authenticated;
grant usage on sequence realtime.messages_id_seq to authenticated;
create function realtime.topic() returns text language sql stable as $$ select current_setting('realtime.topic', true) $$;

create publication supabase_realtime;
-- As on Supabase: extensions live in their own schema, which is on the
-- search path for SQL but not inside functions that pin search_path.
create schema extensions;
grant usage on schema extensions to anon, authenticated, service_role;
create extension if not exists pgcrypto with schema extensions;
alter database postgres set search_path = "$user", public, extensions;

-- Storage (just what the profile media policies use).
create schema storage;
grant usage on schema storage to anon, authenticated, service_role;
create table storage.buckets (id text primary key, name text not null, public boolean default false, file_size_limit bigint, allowed_mime_types text[]);
create table storage.objects (id uuid primary key default gen_random_uuid(), bucket_id text references storage.buckets, name text, owner uuid default auth.uid());
alter table storage.objects enable row level security;
grant select, insert, delete on storage.objects to authenticated;
create function storage.foldername(name text) returns text[] language sql immutable as $$ select (string_to_array(name, '/'))[1:array_length(string_to_array(name, '/'), 1) - 1] $$;
