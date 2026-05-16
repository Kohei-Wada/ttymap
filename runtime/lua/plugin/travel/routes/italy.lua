-- travel.routes.italy — curated Italian itineraries.
--
-- Same shape as `japan.lua`: { country, routes }. Mountains, coast,
-- and Mediterranean islands cover most of what a first-time visitor
-- would book.

return {
    country = "Italy",
    routes = {
        {
            name    = "Classic Italy",
            days    = "10-14 days",
            summary = "Big four — Rome, Florence, Venice, Milan",
            stops = {
                { lon = 12.50, lat = 41.90, zoom = 11, name = "Rome",
                  note = "Rome — Colosseum, Vatican, espresso at every corner" },
                { lon = 11.25, lat = 43.77, zoom = 12, name = "Florence",
                  note = "Florence — Duomo, Uffizi, birthplace of the Renaissance" },
                { lon = 12.34, lat = 45.44, zoom = 12, name = "Venice",
                  note = "Venice — gondolas, mask shops, sinking-island vibes" },
                { lon = 9.19,  lat = 45.46, zoom = 11, name = "Milan",
                  note = "Milan — Duomo, fashion, Last Supper at Santa Maria delle Grazie" },
            },
        },
        {
            name    = "Amalfi Coast + Naples",
            days    = "5-7 days",
            summary = "Cliffside villages, lemon groves, Pompeii",
            stops = {
                { lon = 14.27, lat = 40.85, zoom = 11, name = "Naples",
                  note = "Naples — birthplace of pizza, raw and chaotic charm" },
                { lon = 14.49, lat = 40.75, zoom = 12, name = "Pompeii",
                  note = "Pompeii — frozen 79 AD city under Vesuvius ash" },
                { lon = 14.37, lat = 40.63, zoom = 12, name = "Sorrento",
                  note = "Sorrento — limoncello, cliffs, gateway to the coast" },
                { lon = 14.49, lat = 40.63, zoom = 13, name = "Positano",
                  note = "Positano — pastel houses cascading to the sea" },
                { lon = 14.24, lat = 40.55, zoom = 12, name = "Capri",
                  note = "Capri — Blue Grotto, jet-set island day trip" },
            },
        },
        {
            name    = "Sicily Loop",
            days    = "7 days",
            summary = "Volcano + Greek ruins + arancini",
            stops = {
                { lon = 13.36, lat = 38.12, zoom = 11, name = "Palermo",
                  note = "Palermo — markets, mosaics, street food capital" },
                { lon = 14.02, lat = 38.04, zoom = 12, name = "Cefalù",
                  note = "Cefalù — medieval seaside town under a great rock" },
                { lon = 15.29, lat = 37.85, zoom = 12, name = "Taormina",
                  note = "Taormina — Greek theatre with Mt. Etna in the backdrop" },
                { lon = 15.00, lat = 37.75, zoom = 11, name = "Mt. Etna",
                  note = "Mt. Etna — Europe's most active volcano, hike the craters" },
                { lon = 15.29, lat = 37.07, zoom = 12, name = "Syracuse",
                  note = "Syracuse — Ortigia island, Greek temples, Aretusa fountain" },
            },
        },
    },
}
