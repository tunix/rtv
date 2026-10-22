#!/usr/bin/env python3
"""Generate the committed mock/ dataset for `rtv --mock`.

Credential-free, URL-free (cover URLs point at picsum.photos which only
affects artwork; offline runs keep the placeholder art). Schema mirrors
what src/sample.rs and src/details.rs parse:

  mock/live_categories.json  [{category_id, category_name}]
  mock/live_streams.json     [{stream_id, name, category_id, stream_icon}]
  mock/series_categories.json, mock/vod_categories.json
  mock/series.json           [{series_id, name, category_id, cover, plot,
                               cast, director, genre, rating, releaseDate,
                               youtube_trailer, episode_run_time, backdrop_path}]
  mock/vod_streams.json      [{stream_id, name, category_id, stream_icon, rating}]
  mock/series_info/series_info_<id>.json   {seasons: [...], episodes: {"1": [...]}}
  mock/vod_info/vod_info_<id>.json         {info: {plot, genre, cast, director,
                                                   duration, releasedate}}
  mock/user_info.json        fake creds (shape only, never used by --mock)

Idempotent: regenerate any time with `python3 scripts/generate_mock_dataset.py`.
"""

import json
import os
import random

random.seed(42)
ROOT = os.path.join(os.path.dirname(__file__), "..", "mock")


def write(path, data):
    full = os.path.join(ROOT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w") as f:
        json.dump(data, f, indent=2)
    print(f"wrote {full}")


def cover(seed, w=300, h=450):
    return f"https://picsum.photos/seed/{seed}/{w}/{h}"


# ---------------------------------------------------------------- live
live_cats = ["Sports", "News", "Movies HD", "Documentary", "Kids", "Music"]
live_names = {
    "Sports": ["Sports HD", "Sports 2", "Sports Prime", "Arena Sport", "Stadium Live",
               "Goal TV", "Court Central", "Track & Field", "Fight Night", "Pitch HD"],
    "News": ["News 24", "World News", "Business Daily", "Headline One", "Sky Report",
             "Parliament Live", "Weather Now"],
    "Movies HD": ["Cinema HD", "Cinema Classics", "Action Pack", "Indie Screen"],
    "Documentary": ["Planet Doc", "History Channel", "Deep Blue", "Science Now"],
    "Kids": ["Kids Zone", "Cartoon Club", "Tiny Tots", "Adventure Jr."],
    "Music": ["Hit Music", "Jazz Café", "Rock One", "Classical Live"],
}
live_items = []
sid = 1000
for cid, cat in enumerate(live_cats, 1):
    for name in live_names[cat]:
        sid += 1
        live_items.append({
            "stream_id": sid,
            "num": sid - 1000,
            "name": name,
            "category_id": str(cid),
            "stream_icon": "",
        })

live_categories = [
    {"category_id": str(i), "category_name": c} for i, c in enumerate(live_cats, 1)
]
write("live_categories.json", live_categories)
write("live_streams.json", live_items)

# ---------------------------------------------------------------- series
series_plots = [
    "A detective returns to her hometown and finds every greeting hides a secret.",
    "The crew of a salvage ship stumbles on a wreck nobody wants found.",
    "In a city of endless rain, an archivist discovers files that predict murders.",
    "Two rival chefs share one kitchen and a decade-old feud.",
    "A family vineyard, a forged will, and a summer that will not end.",
    "An astrophysicist hears a signal that answers her in her own voice.",
    "A taxi driver's nightly routes weave a map of the city's buried crimes.",
    "A retiring locksmith receives keys to doors that were never built.",
]
series_cast = [
    "Ela Yilmaz, Marcus Chen, Priya Nair",
    "Tomas Weber, Aisha Bello, Luc Moreau",
    "Hana Sato, Diego Alvarez, Ruth Okonkwo",
    "Marek Nowak, Sofia Rossi, Jin Park",
]
genres = ["Drama", "Thriller", "Mystery", "Comedy", "Sci-Fi", "Crime"]

series_categories = [{"category_id": "1", "category_name": "Scripted"},
                     {"category_id": "2", "category_name": "Documentary Series"}]
series_items = []
for i in range(24):
    s_id = 3000 + i
    name = f"Mock Series #{i + 1:02d}"
    series_items.append({
        "series_id": s_id,
        "num": i + 1,
        "name": name,
        "category_id": "1" if i % 4 else "2",
        "cover": cover(f"rtv-series-{s_id}"),
        "plot": random.choice(series_plots),
        "cast": random.choice(series_cast),
        "director": random.choice(["Aylin Demir", "Jonas Berg", "Carmen Ruiz", "Ken Watanabe"]),
        "genre": random.choice(genres),
        "rating": f"{random.uniform(5.6, 9.4):.1f}",
        "releaseDate": f"{random.randint(2012, 2025)}-{random.randint(1, 12):02d}-01",
        "youtube_trailer": f"https://www.youtube.com/watch?v=mocktrailer{i + 1:02d}",
        "episode_run_time": str(random.choice([28, 32, 42, 45, 50])),
        "backdrop_path": [cover(f"rtv-series-bd-{s_id}", 640, 360)],
    })
write("series_categories.json", series_categories)
write("series.json", series_items)

# series_info dumps: 1-3 seasons, 3-8 episodes each
for s in series_items:
    s_id = s["series_id"]
    n_seasons = random.randint(1, 3)
    seasons = []
    episodes = {}
    total = 0
    for season_no in range(1, n_seasons + 1):
        n_eps = random.randint(3, 8)
        total += n_eps
        seasons.append({
            "air_date": f"{2020 + season_no}-0{season_no}-15",
            "episode_count": n_eps,
            "name": f"Season {season_no}",
            "cover": cover(f"rtv-s{s_id}-e{season_no}", 150, 225),
            "season_number": season_no,
        })
        eps = []
        for ep_no in range(1, n_eps + 1):
            dur = random.randint(22, 55)
            eps.append({
                "id": f"ep_{s_id}_{season_no}_{ep_no}",
                "episode_num": ep_no,
                "title": f"Episode {ep_no}",
                "container_extension": "mkv",
                "info": {
                    "duration": f"{dur // 60}:{dur % 60:02d}:00",
                    "plot": f"S{season_no} E{ep_no} — {random.choice(series_plots)}",
                    "movie_image": cover(f"rtv-s{s_id}-e{season_no}-{ep_no}", 160, 90),
                },
                "season": season_no,
            })
        episodes[str(season_no)] = eps
    write(f"series_info/series_info_{s_id}.json", {"seasons": seasons, "episodes": episodes})

# ---------------------------------------------------------------- vod
vod_plots = [
    "A night-shift baker witnesses a robbery and becomes the only credible witness.",
    "Two strangers swap suitcases at a station and inherit each other's enemies.",
    "The last lighthouse keeper records a shipwreck that officially never happened.",
    "An chess prodigy faces the machine she helped build.",
    "A road movie: one van, four musicians, zero working radios.",
    "A gardener cultivates a species that blooms once a century — tonight.",
]
vod_categories = [{"category_id": "1", "category_name": "Feature Films"},
                  {"category_id": "2", "category_name": "Independent"}]
vod_items = []
for i in range(26):
    v_id = 5000 + i
    vod_items.append({
        "stream_id": v_id,
        "num": i + 1,
        "name": f"Mock Movie #{i + 1:02d}",
        "category_id": "1" if i % 3 else "2",
        "stream_icon": cover(f"rtv-vod-{v_id}"),
        "rating": f"{random.uniform(5.0, 9.2):.1f}",
    })
write("vod_categories.json", vod_categories)
write("vod_streams.json", vod_items)

for v in vod_items:
    v_id = v["stream_id"]
    mins = random.randint(85, 148)
    write(f"vod_info/vod_info_{v_id}.json", {
        "info": {
            "movie_image": cover(f"rtv-vod-big-{v_id}"),
            "duration_secs": mins * 60,
            "duration": f"0:{mins // 60:02d}:{mins % 60:02d}:00",
            "plot": random.choice(vod_plots),
            "cast": random.choice(series_cast),
            "director": random.choice(["Aylin Demir", "Jonas Berg", "Carmen Ruiz"]),
            "genre": random.choice(genres),
            "releasedate": f"{random.randint(2015, 2026)}-{random.randint(1, 12):02d}-{random.randint(1, 28):02d}",
            "rating": v["rating"],
        }
    })

# ---------------------------------------------------------------- user_info
# Shape-only placeholder for the real-mode details layer; --mock never reads
# it and real mode without samples/ degrades to graceful toasts.
write("user_info.json", {
    "user_info": {"username": "mock", "password": "mock", "status": "Active"},
    "server_info": {"url": "mock.invalid", "port": 80, "server_protocol": "http"},
})

print("done")
