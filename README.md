# Bandcamp Daily Fetcher

The [Bandcamp Daily blog](https://daily.bandcamp.com/) is a fantastic resource
for discovering new music. I follow it in my RSS reader, but I can never keep up
with reading (and listening to) new posts as they come out.

I wrote this tool to help myself out.

**B**and**c**amp **D**aily **F**etcher (`bcdf`) is a program for following and 
consuming music from the Bandcamp Daily blog. It scans the blog's RSS feed for 
posts and creates Spotify playlists for them. It can also download the songs 
straight from Bandcamp for listening in a local media player.

```
$ bcdf --help
Follow the Bandcamp Daily blog: scrape posts, download mp3s, and build Spotify playlists

Usage: bcdf [OPTIONS] <COMMAND>

Commands:
  scan      Scrape new posts, download their mp3s, and update Spotify playlists
  mp3       Manage downloaded mp3 files
  post      View/manage bandcamp blog posts
  track     View and manage tracks
  playlist  View and manage Spotify playlists
  spotify   Spotify management and debug actions
  help      Print this message or the help of the given subcommand(s)

Options:
      --data-dir <DIR>     Base directory for storing state and downloaded mp3 files [default: ./data]
  -v, --verbose...         Increase log verbosity: -v for debug, -vv for trace
      --log-level <LEVEL>  Log level
      --color <WHEN>       When to colorize log output [default: auto] [possible values: auto, always, never]
  -h, --help               Print help
  -V, --version            Print version

```

## status

I created `bcdf` for my own personal use. While it does work, it is fairly
unpolished and inelegant. At this moment I have no concrete plans to make it
more usable/suitable to a wider audience (though I guess this could change), so
manage your expectations accordingly. I'm generally open to receiving outside
contributions, but please make an issue first to discuss.
