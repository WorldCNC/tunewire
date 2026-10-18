# Changelog

Each release's section is used as its GitHub release notes and shown in the
app's update and "What's new" dialogs. Keep entries short and user-facing.

## Unreleased

**Your library on all your devices**
- Turn on Settings → Account → Sync my library: your playlists and Liked Songs (with their covers, folders and pins) show up on every computer you sign in to. The online copies are private.
- Changes made offline go out when you're back, even after closing the app
- Playlists you share or collaborate on now stay in sync between your own computers too
- New: a quick tour of what an account adds (Listen Together, friends, your library everywhere), shown once if you're not signed in

**Playback**
- Seeking in SoundCloud songs no longer waits for everything before that point
- After a seek, the old audio stops right away instead of playing on for a moment
- The progress bar stays right when a seek fails
- When headphones or a USB/Bluetooth device disappear, playback continues on the current output instead of sitting silent
- Songs keep starting fast if yt-dlp's helper process crashes
- YouTube songs start about a second sooner: Tunewire's own yt-dlp copy runs on Python when it's installed (Linux, macOS; switched automatically), and looking up a song skips loading its web page
- Songs no longer break off on slow connections

**Search and profiles**
- Search suggestions now show songs, artists and albums with their pictures while you type; Enter plays a song (Ctrl+Enter adds it to the queue)
- Set your own profile picture and banner, GIFs included. Your Discord picture and banner are only used when you join; after that they change only when you change them
- Songs whose YouTube picture is gone show a clean placeholder instead of a gray box

**Faster and sturdier**
- Searching All no longer waits about 2 seconds extra when YouTube has nothing to show for a song
- Searches with Spotify stay fast after the app sat idle for an hour
- Listen Together, listening along and notifications catch up on anything they missed while the connection was down
- Searching All shows its results within about 5 seconds even when one source is slow
- Liking songs, editing playlists and starting songs no longer wait for the disk
- Signed out, only names and pictures are visible: bios, pronouns and what you share need a signed-in account (share links still work)

## v2.0.1

A big one: a new social side, a safer online service, a Mac version, and a smoother, sturdier app.

**Friends**
- Profiles with your Discord banner, colors and avatar decoration, badges, a bio and pronouns, what you're playing, your top artists and songs (if you share them), mutual friends and a taste match
- Send a song to a friend with a note; it lands in their Inbox, where they can react
- Listen along with a friend: your player follows theirs, no session needed
- See who's online, what they're playing and how far in, plus a weekly leaderboard
- Notifications for follow requests, songs sent to you, changes to shared playlists and friends starting a session

**Listening together**
- Listen Together, rebuilt: only the host controls playback; the host can hand over, remove people and approve suggested songs; everyone can chat and vote to skip; you can keep your own room link
- Shared playlists show whether they're synced and who's editing, with a Retry when something didn't sync; renames and covers reach everyone

**Privacy and safety**
- Accounts sign in with Discord (guest accounts are gone)
- People need your approval to follow you; sharing what you play and your listening stats is off until you turn it on
- Links ask before joining a playlist or session or playing a song
- Error reports are off unless you turn them on

**New**
- Scrobble to Last.fm and ListenBrainz
- Back up your whole library to one file and restore it; a damaged library repairs itself from a daily copy
- Theme and plugin gallery
- Undo after deleting a playlist, removing songs or clearing history
- A new full-screen lyrics view with the cover and playback controls
- New app icon and website
- Tunewire for Mac (Apple Silicon and Intel)

**Smoother**
- No more flickering on Linux with NVIDIA graphics
- Smoother scrolling in long playlists; no blinking covers, menus or backgrounds
- Spotify import works again: sign-in no longer gets stuck, and your playlists and Liked Songs are read without hitting Spotify's rate limits
- Imported Spotify songs start sooner
- Keyboard navigation in song lists, screen reader labels, and a compact player bar in narrow windows

## v1.0.4

- Accounts: sign in (or continue as a guest) to share and listen together; your music still lives on your computer
- Share a song or a playlist with a link; people can open it, save it, or follow it to get your updates
- Collaborative playlists: invite friends, everyone adds, removes and reorders songs, changes show up live
- Listen Together: start a session, send the link, everyone hears the same song at the same moment; guests add songs and send reactions
- Friends: see what people you follow are playing, find people, profile pages, compare your Wrapped
- Much faster: search across all sources in about half a second, SoundCloud songs start in under half a second, artist and album pages in under a second
- Spotify imports: your whole library (Liked Songs and playlists) or someone's public playlists, now without developer keys
- Far less memory: right-sized covers, long lists load as you scroll, no live blur
- Smoother: no flicker when animations end or when reordering a playlist; download button fills up as a playlist downloads
- Error reports help fix problems faster (can be turned off in Settings → Privacy)
- Updates are checked every hour

## v1.0.3

- New look: your own title bar, Your Library sidebar (pins, folders, filters, sort), big playlist headers in the cover's color, Spotify-style song rows and player bar
- Now Playing view with a big cover, About the artist and what's next; full screen player
- Artist and album pages load in under a second, with Popular, Discography and Fans also like
- Search shows a top result, songs, artists, albums and playlists; Browse all moods and genres
- Import your whole Spotify library (Liked Songs + playlists, private ones too), Apple Music links and CSV files
- Imported Spotify songs are matched to YouTube Music in the background, so they start instantly
- Select several songs (Shift/Ctrl-click) and drag songs onto playlists, the queue, or to reorder
- Search and sort inside playlists, date added, smart shuffle, shuffle mode, sleep timer
- Edit playlist details: name, description and your own cover picture
- If yt-dlp is missing, one click installs it
- Linux builds are back (AppImage, .deb, .rpm)

## v1.0.2

- Home: a new start page with Daily Mixes, Discover Weekly (new songs every Monday), On Repeat and Forgotten favorites
- Your stats: minutes listened, top artists and songs, listening streaks, time of day, for 4 weeks, 6 months, this year or all time
- Wrapped: your year in music as a story, with a share image and a "top songs" playlist
- Media keys and the Windows media overlay now control Tunewire (play/pause, next, previous, seek)
- Updates download in the background and install when you restart
- Playlist recommendations, with "Because of …" reasons
- New Ambient look: soft background from the playing song's cover, panels and smoother motion
- Songs start much faster, often almost instantly
