# Bandcamp Daily Fetcher

The [Bandcamp Daily blog](https://daily.bandcamp.com/) is a fantastic resource
for discovering new music. I follow it in my RSS reader, but I can never keep on
top of reading (and listening) to new posts.

So I wrote this tool to help me out.

The Bandcamp Daily Fetcher (`bcdf`) is a program for following and consuming
music from the Bandcamp Daily blog. It scans the blog's RSS feed for posts and
creates Spotify playlists for them. It can also download the songs straight from
Bandcamp for listening in a local media player.


```
$ bcdf --help
Usage: bcdf [OPTIONS]

Options:
      --download-to <PATH>  Base directory for storing downloaded content [default: ./data]
      --no-download         Don't download anything
      --no-spotify          Don't create Spotify playlists
  -h, --help                Print help
  -V, --version             Print version
```

## status

I wrote `bcdf` for my own personal use. It works, but there are warts. I don't
anticipate putting much effort into UX improvements/documentation to make it
suitable for a wider audience, so manage your expectations accordingly.