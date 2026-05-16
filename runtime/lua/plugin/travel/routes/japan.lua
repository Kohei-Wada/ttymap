-- travel.routes.japan — curated Japan itineraries for foreign visitors.
--
-- Each route lists 5-6 stops. The `note` field is used twice:
--   1. As the secondary line in the sidebar list view (one-liner about
--      the city or its attraction).
--   2. As the per-stop notify popup during the animated tour.
--
-- Coordinates are city-centric where the route lands a tourist (e.g.
-- Hakone is the Tougendai cable-car area, not the township office;
-- Jigokudani is the snow monkey park itself, not Yamanouchi station).
-- Zoom levels are tuned so the named place is recognisable in the
-- terminal grid — denser cities get z=10, single-attraction stops
-- get z=11/12.

return {
    country = "Japan",
    routes = {
    {
        name    = "Golden Route",
        days    = "10-14 days",
        summary = "First-timer classic — Tokyo, Mt. Fuji, Kyoto, Nara, Osaka",
        stops = {
            { lon = 139.69, lat = 35.69, zoom = 10, name = "Tokyo",
              note = "Tokyo — neon nights, ramen mornings, ancient shrines" },
            { lon = 139.03, lat = 35.23, zoom = 11, name = "Hakone",
              note = "Hakone — onsen + Mt. Fuji on a clear day" },
            { lon = 135.77, lat = 35.01, zoom = 11, name = "Kyoto",
              note = "Kyoto — 1000-year capital, Fushimi Inari's red gates" },
            { lon = 135.83, lat = 34.69, zoom = 12, name = "Nara",
              note = "Nara — friendly deer, the giant Buddha at Todai-ji" },
            { lon = 135.50, lat = 34.69, zoom = 11, name = "Osaka",
              note = "Osaka — takoyaki, neon Dotonbori, Japan's kitchen" },
        },
    },
    {
        name    = "Hokkaido Winter Loop",
        days    = "7 days",
        summary = "Snow + onsen + seafood — Sapporo to Hakodate",
        stops = {
            { lon = 141.35, lat = 43.06, zoom = 10, name = "Sapporo",
              note = "Sapporo — snow festival, miso ramen, Susukino nightlife" },
            { lon = 140.99, lat = 43.19, zoom = 12, name = "Otaru",
              note = "Otaru — gas-lamp canal at dusk, glassware, sushi" },
            { lon = 140.69, lat = 42.85, zoom = 11, name = "Niseko",
              note = "Niseko — world-class powder, ski + onsen combo" },
            { lon = 140.85, lat = 42.62, zoom = 11, name = "Lake Toya",
              note = "Lake Toya — caldera lake, hot springs by the water" },
            { lon = 140.73, lat = 41.77, zoom = 11, name = "Hakodate",
              note = "Hakodate — million-dollar night view from Mt. Hakodate" },
        },
    },
    {
        name    = "Kyushu Onsen Trail",
        days    = "7 days",
        summary = "Active volcanoes + steaming hot springs",
        stops = {
            { lon = 130.40, lat = 33.59, zoom = 10, name = "Fukuoka",
              note = "Fukuoka — Hakata ramen, riverside yatai food stalls" },
            { lon = 131.50, lat = 33.28, zoom = 11, name = "Beppu",
              note = "Beppu — eight steaming 'hells' of coloured hot springs" },
            { lon = 131.36, lat = 33.26, zoom = 11, name = "Yufuin",
              note = "Yufuin — quiet onsen town, art galleries, soft pudding" },
            { lon = 131.10, lat = 32.88, zoom = 11, name = "Mt. Aso",
              note = "Mt. Aso — peer into one of the world's largest active calderas" },
            { lon = 130.55, lat = 31.59, zoom = 11, name = "Kagoshima",
              note = "Kagoshima — Sakurajima volcano puffing across the bay" },
        },
    },
    {
        name    = "Snow Monkey + Japan Alps",
        days    = "7 days",
        summary = "Mountain heritage — bathing macaques, gassho farmhouses",
        stops = {
            { lon = 139.69, lat = 35.69, zoom = 10, name = "Tokyo",
              note = "Start: Tokyo — board a Shinkansen north" },
            { lon = 138.46, lat = 36.73, zoom = 12, name = "Jigokudani",
              note = "Jigokudani — wild macaques bathing in a hot spring" },
            { lon = 137.97, lat = 36.24, zoom = 11, name = "Matsumoto",
              note = "Matsumoto — original 16th-century black 'Crow' castle" },
            { lon = 137.25, lat = 36.14, zoom = 11, name = "Takayama",
              note = "Takayama — Edo-era streets, sake breweries" },
            { lon = 136.91, lat = 36.26, zoom = 12, name = "Shirakawa-go",
              note = "Shirakawa-go — UNESCO gassho-zukuri farmhouses" },
            { lon = 136.66, lat = 36.56, zoom = 11, name = "Kanazawa",
              note = "Kanazawa — Kenroku-en garden, gold leaf, geisha district" },
        },
    },
    {
        name    = "Hiroshima + Setouchi",
        days    = "5 days",
        summary = "West Japan highlights — peace, castles, floating torii",
        stops = {
            { lon = 135.50, lat = 34.69, zoom = 10, name = "Osaka",
              note = "Start: Osaka — board the Sanyo Shinkansen west" },
            { lon = 134.69, lat = 34.84, zoom = 12, name = "Himeji",
              note = "Himeji — gleaming white 'White Heron' castle" },
            { lon = 132.46, lat = 34.39, zoom = 11, name = "Hiroshima",
              note = "Hiroshima — Peace Memorial Park, okonomiyaki" },
            { lon = 132.32, lat = 34.30, zoom = 12, name = "Miyajima",
              note = "Miyajima — floating torii gate at high tide" },
            { lon = 132.18, lat = 34.17, zoom = 12, name = "Iwakuni",
              note = "Iwakuni — five-arched Kintai-kyo bridge" },
        },
    },
    },
}
