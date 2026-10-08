//! v22 Game servers: the user picks ONE game, presses Start, and every server region of that game is pinged once a second
//! until Stop (or another game is picked, or the tab is left). Nothing runs when the tab opens.
//!
//! The regions are the drawing's (menu-v22 GSV) where a pingable address exists. Where a publisher publishes nothing,
//! a region is measured with a STAND-IN in the same city (flagged `stand_in`, the page shows "≈" in the pill's tip):
//! - Valve SDR relays (CS2 / Dota 2's OWN relays; for other games a same-city stand-in). Addresses = the first relay of
//!   each POP in api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730, read 2026-10-08 (official, undocumented).
//! - AWS GameLift UDP ping beacons `gamelift-ping.<region>.api.aws:7770` (official AWS: docs.aws.amazon.com/
//!   gameliftservers/latest/developerguide/reference-udp-ping-beacons.html). AWS's limit is 3 pings / s per sender port;
//!   every probe here uses a fresh socket and one try per second.
//! - Epic's own Fortnite ping hosts `ping-<region>.ds.on.epicgames.com` (ping-eu: Epic help; the other region names follow
//!   the same pattern - guess, measured by the network-show example).
//!
//! A region with no known address at all (VALORANT / League "Istanbul") is listed without a target: the page shows "—".

use crate::gameservers::{GameServer, Probe, Target};

/// One region row of a game: the server (its targets) + the place line under the name ("Germany", "Virginia").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub server: GameServer,
    pub place: String,
}

/// One game of the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// the drawing's id: "val", "cs2", "fn", "rl", "lol", "apex", "dota"
    pub id: &'static str,
    pub name: &'static str,
    pub publisher: &'static str,
    pub regions: Vec<Region>,
}

const SDR_SRC: &str = "Valve SDR relay, api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730 (read 2026-10-08)";
const AWS_SRC: &str = "AWS GameLift UDP ping beacon (official AWS docs, port 7770)";
const EPIC_SRC: &str = "Epic Fortnite ping host (ping-eu: Epic help 'latency and ping troubleshooting'; other regions: same pattern, guess)";

/// The first relay address of a Valve SDR POP (GetSDRConfig appid 730, read 2026-10-08 by Lane B2: every POP with a relay,
/// China's partner POPs and the duplicate Datapacket / second-site POPs left out).
fn sdr_ip(pop: &str) -> Option<&'static str> {
    Some(match pop {
        "ams" => "155.133.248.36",
        "atl" => "162.254.199.170",
        "bom" => "155.133.224.20",
        "dfw" => "162.254.194.37",
        "dxb" => "185.25.183.163",
        "eze" => "155.133.255.98",
        "fra" => "155.133.226.68",
        "gru" => "155.133.227.35",
        "gum" => "185.25.180.18",
        "hkg" => "103.28.54.163",
        "iad" => "162.254.192.88",
        "jnb" => "155.133.238.178",
        "lax" => "162.254.195.52",
        "lhr" => "162.254.196.66",
        "lim" => "155.133.244.35",
        "maa" => "155.133.225.18",
        "mad" => "155.133.246.34",
        "ord" => "162.254.193.71",
        "par" => "185.25.182.18",
        "scl" => "155.133.249.163",
        "sea" => "205.196.6.135",
        "seo" => "146.66.152.36",
        "sgp" => "103.10.124.116",
        "sto" => "162.254.198.41",
        "syd" => "103.10.125.20",
        "tyo" => "45.121.184.5",
        "vie" => "146.66.155.66",
        "waw" => "155.133.230.98",
        _ => return None,
    })
}

/// Valve's whole relay network (CS2 and Dota 2 are played through it): (POP, region name, place), Europe first, then the
/// Middle East / Africa / Asia / Oceania, then the Americas.
const SDR_POPS: [(&str, &str, &str); 28] = [
    ("vie", "Vienna", "Austria"),
    ("fra", "Frankfurt", "Germany"),
    ("ams", "Amsterdam", "Netherlands"),
    ("waw", "Warsaw", "Poland"),
    ("sto", "Stockholm", "Sweden"),
    ("mad", "Madrid", "Spain"),
    ("lhr", "London", "UK"),
    ("par", "Paris", "France"),
    ("dxb", "Dubai", "UAE"),
    ("jnb", "Johannesburg", "South Africa"),
    ("bom", "Mumbai", "India"),
    ("maa", "Chennai", "India"),
    ("sgp", "Singapore", "Singapore"),
    ("hkg", "Hong Kong", "China"),
    ("tyo", "Tokyo", "Japan"),
    ("seo", "Seoul", "South Korea"),
    ("syd", "Sydney", "Australia"),
    ("gum", "Guam", "USA"),
    ("iad", "Virginia", "USA"),
    ("atl", "Atlanta", "USA"),
    ("ord", "Chicago", "USA"),
    ("dfw", "Dallas", "USA"),
    ("lax", "Los Angeles", "USA"),
    ("sea", "Seattle", "USA"),
    ("gru", "São Paulo", "Brazil"),
    ("eze", "Buenos Aires", "Argentina"),
    ("lim", "Lima", "Peru"),
    ("scl", "Santiago", "Chile"),
];

/// How one region is measured.
enum How {
    /// the game's own Valve relay
    ValveOwn(&'static str),
    /// a Valve relay in the same city (stand-in)
    Valve(&'static str),
    /// an AWS GameLift beacon (stand-in)
    Aws(&'static str),
    /// a host of the game's own (ICMP)
    Own(&'static str, &'static str),
}

fn target(h: &How) -> Option<Target> {
    Some(match h {
        How::ValveOwn(p) => Target { host: sdr_ip(p)?.into(), probes: vec![Probe::Icmp], stand_in: false, source: format!("{SDR_SRC}, POP '{p}'") },
        How::Valve(p) => Target {
            host: sdr_ip(p)?.into(),
            probes: vec![Probe::Icmp],
            stand_in: true,
            source: format!("{SDR_SRC}, POP '{p}' - same-city stand-in"),
        },
        How::Aws(r) => Target {
            host: format!("gamelift-ping.{r}.api.aws"),
            probes: vec![Probe::Udp(7770)],
            stand_in: true,
            source: format!("{AWS_SRC}, {r} - same-city stand-in"),
        },
        How::Own(host, src) => Target { host: (*host).into(), probes: vec![Probe::Icmp], stand_in: false, source: (*src).into() },
    })
}

fn game(id: &'static str, name: &'static str, publisher: &'static str, rows: Vec<(&str, &str, Vec<How>)>) -> Game {
    let regions = rows
        .into_iter()
        .enumerate()
        .map(|(i, (region, place, hows))| Region {
            server: GameServer {
                id: format!("{id}.{i}"),
                game: name.into(),
                publisher: publisher.into(),
                region: region.into(),
                targets: hows.iter().filter_map(target).collect(),
            },
            place: place.into(),
        })
        .collect();
    Game { id, name, publisher, regions }
}

/// A Valve game's rows: every relay POP, measured on its own relay.
fn valve_rows() -> Vec<(&'static str, &'static str, Vec<How>)> {
    SDR_POPS.iter().map(|(p, region, place)| (*region, *place, vec![How::ValveOwn(p)])).collect()
}

/// The picker's games, in the drawing's order, each with EVERY server region the game has (the owner Oct 8: only close servers
/// were listed "rather than all in the game"), Europe first (the drawing's own rows keep their order at the top).
/// AWS beacons resolved by DNS 2026-10-08 (Lane B2); a region with no address of its own and no stand-in in the same city
/// has no target (the page shows "—").
pub fn games() -> Vec<Game> {
    use How::*;
    const EPIC: &str = EPIC_SRC;
    vec![
        // Riot publishes no addresses (support.riotgames.com server-select page); its shards run on AWS (re:Invent 2021 GAM302)
        // - which AWS city each region is = guess. AWS first, Valve's same-city relay as the fallback. Regions: Riot's list
        // (NA, LATAM, BR, EU, MENA, AP, KR), read 2026-10-08.
        game(
            "val",
            "VALORANT",
            "Riot Games",
            vec![
                ("Frankfurt", "Germany", vec![Aws("eu-central-1"), Valve("fra")]),
                ("Paris", "France", vec![Aws("eu-west-3"), Valve("par")]),
                ("London", "UK", vec![Aws("eu-west-2"), Valve("lhr")]),
                ("Stockholm", "Sweden", vec![Aws("eu-north-1"), Valve("sto")]),
                ("Warsaw", "Poland", vec![Valve("waw")]),
                ("Madrid", "Spain", vec![Aws("eu-south-2"), Valve("mad")]),
                ("Istanbul", "Turkey", vec![]),
                ("Bahrain", "Middle East", vec![Aws("me-south-1"), Valve("dxb")]),
                ("Dubai", "UAE", vec![Valve("dxb")]),
                ("Virginia", "US East", vec![Aws("us-east-1"), Valve("iad")]),
                ("Atlanta", "US East", vec![Valve("atl")]),
                ("Chicago", "US Central", vec![Valve("ord")]),
                ("Texas", "US Central", vec![Valve("dfw")]),
                ("N. California", "US West", vec![Aws("us-west-1"), Valve("lax")]),
                ("Oregon", "US West", vec![Aws("us-west-2"), Valve("sea")]),
                ("Mexico City", "Mexico", vec![Aws("mx-central-1")]),
                ("Miami", "USA", vec![]),
                ("Santiago", "Chile", vec![Valve("scl")]),
                ("São Paulo", "Brazil", vec![Aws("sa-east-1"), Valve("gru")]),
                ("Mumbai", "India", vec![Aws("ap-south-1"), Valve("bom")]),
                ("Singapore", "Singapore", vec![Aws("ap-southeast-1"), Valve("sgp")]),
                ("Hong Kong", "China", vec![Aws("ap-east-1"), Valve("hkg")]),
                ("Tokyo", "Japan", vec![Aws("ap-northeast-1"), Valve("tyo")]),
                ("Seoul", "South Korea", vec![Aws("ap-northeast-2"), Valve("seo")]),
                ("Sydney", "Australia", vec![Aws("ap-southeast-2"), Valve("syd")]),
            ],
        ),
        // Valve's own relays, all of them (Luxembourg and Helsinki of the drawing have no relay address today).
        game("cs2", "Counter-Strike 2", "Valve", valve_rows()),
        game(
            "fn",
            "Fortnite",
            "Epic Games",
            vec![
                ("Europe", "Frankfurt", vec![Own("ping-eu.ds.on.epicgames.com", EPIC), Aws("eu-central-1")]),
                // Bahrain: Epic's host and the AWS beacon resolve but did not answer from the test PC (network-show 2026-10-08):
                // Valve's Dubai relay is the Middle East stand-in after them
                ("Middle East", "Bahrain", vec![Own("ping-me.ds.on.epicgames.com", EPIC), Aws("me-south-1"), Valve("dxb")]),
                ("NA East", "Virginia", vec![Own("ping-nae.ds.on.epicgames.com", EPIC), Aws("us-east-1")]),
                ("NA Central", "Texas", vec![Own("ping-nac.ds.on.epicgames.com", EPIC), Valve("dfw")]),
                // NA West: Oregon = guess (Epic says only "NA West")
                ("NA West", "Oregon", vec![Own("ping-naw.ds.on.epicgames.com", EPIC), Aws("us-west-2")]),
                ("Brazil", "São Paulo", vec![Own("ping-br.ds.on.epicgames.com", EPIC), Aws("sa-east-1")]),
                ("Asia", "Tokyo", vec![Own("ping-asia.ds.on.epicgames.com", EPIC), Aws("ap-northeast-1")]),
                ("Oceania", "Sydney", vec![Own("ping-oce.ds.on.epicgames.com", EPIC), Aws("ap-southeast-2")]),
            ],
        ),
        // Psyonix publishes no addresses: same-city stand-ins. Regions: Epic's "regional restrictions for Rocket League" page.
        game(
            "rl",
            "Rocket League",
            "Psyonix",
            vec![
                ("Europe", "Amsterdam", vec![Valve("ams")]),
                ("Middle East", "Bahrain", vec![Aws("me-south-1"), Valve("dxb")]),
                ("US East", "Virginia", vec![Aws("us-east-1"), Valve("iad")]),
                ("South Africa", "Johannesburg", vec![Valve("jnb")]),
                ("US West", "California", vec![Aws("us-west-1"), Valve("lax")]),
                ("South America", "São Paulo", vec![Aws("sa-east-1"), Valve("gru")]),
                ("Asia SE", "Singapore", vec![Aws("ap-southeast-1"), Valve("sgp")]),
                ("Oceania", "Sydney", vec![Aws("ap-southeast-2"), Valve("syd")]),
                ("US Central", "Texas", vec![Valve("dfw")]),
                ("Asia Mainland", "Hong Kong", vec![Aws("ap-east-1"), Valve("hkg")]),
                ("Asia East", "Tokyo", vec![Aws("ap-northeast-1"), Valve("tyo")]),
                ("India", "Mumbai", vec![Aws("ap-south-1"), Valve("bom")]),
            ],
        ),
        game(
            "lol",
            "League of Legends",
            "Riot Games",
            vec![
                (
                    "EU West",
                    "Amsterdam",
                    vec![Own("104.160.141.3", "community-known EUW ping address (Riot-owned range per db-ip.com)"), Valve("ams")],
                ),
                ("EU Nordic & East", "Frankfurt", vec![Valve("fra")]),
                ("Turkey", "Istanbul", vec![]),
                (
                    "North America",
                    "Chicago",
                    vec![Own("104.160.131.3", "community-known NA ping address (Riot-owned range per db-ip.com)"), Valve("ord")],
                ),
                ("Middle East", "Bahrain", vec![Aws("me-south-1"), Valve("dxb")]),
                ("Russia", "Moscow", vec![]),
                ("Brazil", "São Paulo", vec![Aws("sa-east-1"), Valve("gru")]),
                ("Latin America North", "Miami", vec![]),
                ("Latin America South", "Santiago", vec![Valve("scl")]),
                ("Oceania", "Sydney", vec![Aws("ap-southeast-2"), Valve("syd")]),
                ("Japan", "Tokyo", vec![Aws("ap-northeast-1"), Valve("tyo")]),
                ("Korea", "Seoul", vec![Aws("ap-northeast-2"), Valve("seo")]),
                ("Southeast Asia", "Singapore", vec![Aws("ap-southeast-1"), Valve("sgp")]),
            ],
        ),
        // EA publishes no addresses (2019: Multiplay + Google Cloud): same-city stand-ins for its data-centre cities.
        game(
            "apex",
            "Apex Legends",
            "EA",
            vec![
                ("Frankfurt", "Germany", vec![Aws("eu-central-1"), Valve("fra")]),
                ("London", "UK", vec![Aws("eu-west-2"), Valve("lhr")]),
                ("Paris", "France", vec![Aws("eu-west-3"), Valve("par")]),
                ("Amsterdam", "Netherlands", vec![Valve("ams")]),
                ("Warsaw", "Poland", vec![Valve("waw")]),
                ("Madrid", "Spain", vec![Aws("eu-south-2"), Valve("mad")]),
                ("Stockholm", "Sweden", vec![Aws("eu-north-1"), Valve("sto")]),
                ("Bahrain", "Middle East", vec![Aws("me-south-1"), Valve("dxb")]),
                ("Johannesburg", "South Africa", vec![Valve("jnb")]),
                ("Virginia", "US East", vec![Aws("us-east-1"), Valve("iad")]),
                ("Ohio", "US East", vec![Aws("us-east-2")]),
                ("Chicago", "US Central", vec![Valve("ord")]),
                ("Texas", "US Central", vec![Valve("dfw")]),
                ("N. California", "US West", vec![Aws("us-west-1"), Valve("lax")]),
                ("Oregon", "US West", vec![Aws("us-west-2"), Valve("sea")]),
                ("São Paulo", "Brazil", vec![Aws("sa-east-1"), Valve("gru")]),
                ("Mumbai", "India", vec![Aws("ap-south-1"), Valve("bom")]),
                ("Singapore", "Singapore", vec![Aws("ap-southeast-1"), Valve("sgp")]),
                ("Hong Kong", "China", vec![Aws("ap-east-1"), Valve("hkg")]),
                ("Tokyo", "Japan", vec![Aws("ap-northeast-1"), Valve("tyo")]),
                ("Seoul", "South Korea", vec![Aws("ap-northeast-2"), Valve("seo")]),
                ("Sydney", "Australia", vec![Aws("ap-southeast-2"), Valve("syd")]),
            ],
        ),
        game("dota", "Dota 2", "Valve", valve_rows()),
    ]
}

/// The region with the lowest answer ("Best" tag), if any answered. `(id, ms)` pairs.
pub fn best<'a>(results: impl IntoIterator<Item = (&'a str, Option<u32>)>) -> Option<&'a str> {
    results.into_iter().filter_map(|(id, ms)| ms.map(|m| (id, m))).min_by_key(|(_, m)| *m).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seven_games_in_the_drawings_order_with_every_region() {
        let g = games();
        let ids: Vec<&str> = g.iter().map(|g| g.id).collect();
        assert_eq!(ids, ["val", "cs2", "fn", "rl", "lol", "apex", "dota"]);
        let counts: Vec<usize> = g.iter().map(|g| g.regions.len()).collect();
        // every region of each game (the owner Oct 8), not only the drawing's European ones
        assert_eq!(counts, [25, 28, 8, 12, 13, 22, 28]);
        // the drawing's rows stay first, in its order
        let val: Vec<&str> = g[0].regions.iter().take(7).map(|r| r.server.region.as_str()).collect();
        assert_eq!(val, ["Frankfurt", "Paris", "London", "Stockholm", "Warsaw", "Madrid", "Istanbul"]);
        for (i, want) in [(1, "Vienna"), (6, "Dota 2")] {
            let first = if want == "Dota 2" { g[i].name } else { g[i].regions[0].server.region.as_str() };
            assert_eq!(first, want);
        }
    }

    #[test]
    fn only_regions_with_no_address_anywhere_lack_a_target_and_valve_games_use_their_own_relays() {
        // no published address and no relay / beacon in the same city
        let none = ["Istanbul", "Miami", "Moscow"];
        for g in games() {
            let mut ids = std::collections::HashSet::new();
            for r in &g.regions {
                let no_addr = none.contains(&r.server.region.as_str()) || none.contains(&r.place.as_str());
                assert_eq!(r.server.targets.is_empty(), no_addr, "{} {}", g.name, r.server.region);
                if g.publisher == "Valve" {
                    assert!(r.server.targets.iter().all(|t| !t.stand_in), "{} {}", g.name, r.server.region);
                }
                assert!(ids.insert(r.server.id.clone()));
            }
        }
    }

    #[test]
    fn best_is_the_lowest_answer() {
        assert_eq!(best([("a", Some(30)), ("b", None), ("c", Some(12))]), Some("c"));
        assert_eq!(best([("a", None)]), None);
    }
}
