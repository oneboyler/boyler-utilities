//! The EU game-server list - one region per game, the DESIGN §3.11 rows.
//! Research 2026-10-08 (reports/order_008.md "Game servers"): only Fortnite, CS2 and Dota 2 have public addresses
//! of the game's own servers / relays. VALORANT, Rocket League, Apex and (from the test network) League have none
//! that answers, so they use a STAND-IN: a public endpoint in the same city, flagged so the menu shows "≈".

use crate::gameservers::{GameServer, Probe, Target};

struct T {
    host: &'static str,
    probes: &'static [Probe],
    stand_in: bool,
    source: &'static str,
}

struct Row {
    id: &'static str,
    game: &'static str,
    publisher: &'static str,
    region: &'static str,
    targets: &'static [T],
}

const ICMP: &[Probe] = &[Probe::Icmp];

const VALVE_SDR: &str = "Valve SDR relay list, api.steampowered.com/ISteamApps/GetSDRConfig/v1/?appid=730 (official, \
undocumented; relays answered ICMP 2026-10-08)";

/// AWS GameLift UDP ping beacon, Frankfurt (official AWS:
/// docs.aws.amazon.com/gameliftservers/latest/developerguide/reference-udp-ping-beacons.html), then the AWS Frankfurt
/// DynamoDB endpoint (ICMP / TCP 443 answer).
const AWS_FRA: [T; 2] = [
    T {
        host: "gamelift-ping.eu-central-1.api.aws",
        probes: &[Probe::Udp(7770)],
        stand_in: true,
        source: "AWS GameLift UDP ping beacon eu-central-1 (Frankfurt), official AWS",
    },
    T {
        host: "dynamodb.eu-central-1.amazonaws.com",
        probes: &[Probe::Icmp, Probe::Tcp(443)],
        stand_in: true,
        source: "AWS eu-central-1 (Frankfurt) service endpoint, ip-ranges.amazonaws.com",
    },
];

const VALVE_AMS: T = T {
    host: "155.133.248.36",
    probes: ICMP,
    stand_in: true,
    source: "Valve SDR relay 'ams' (Amsterdam), GetSDRConfig - used only as an Amsterdam location stand-in",
};

const ROWS: &[Row] = &[
    Row {
        id: "valorant",
        game: "VALORANT",
        publisher: "Riot Games",
        region: "Frankfurt",
        // Riot publishes no server addresses (support.riotgames.com .../valorant/support-tools/server-select);
        // VALORANT runs partly on AWS (re:Invent 2021 GAM302) - that it is AWS Frankfurt is a guess.
        targets: &AWS_FRA,
    },
    Row {
        id: "cs2",
        game: "Counter-Strike 2",
        publisher: "Valve",
        region: "Vienna",
        targets: &[
            T { host: "146.66.155.66", probes: ICMP, stand_in: false, source: VALVE_SDR },
            T { host: "146.66.155.67", probes: ICMP, stand_in: false, source: VALVE_SDR },
        ],
    },
    Row {
        id: "fortnite",
        game: "Fortnite",
        publisher: "Epic Games",
        region: "Europe",
        targets: &[T {
            host: "ping-eu.ds.on.epicgames.com",
            probes: ICMP,
            stand_in: false,
            source: "Epic help 'Fortnite latency and ping troubleshooting' (official; resolves to AWS Frankfurt / \
                     Paris / London - every address is pinged, the lowest counts)",
        }],
    },
    Row {
        id: "rocket-league",
        game: "Rocket League",
        publisher: "Psyonix",
        region: "Amsterdam",
        // Hosting unpublished (unclear); EU = Netherlands per community.
        targets: &[VALVE_AMS],
    },
    Row {
        id: "league",
        game: "League of Legends",
        publisher: "Riot Games",
        region: "EU West",
        targets: &[
            T {
                host: "104.160.141.3",
                probes: &[Probe::Icmp],
                stand_in: false,
                source: "community-known EUW ping address (Riot-owned range per db-ip.com); did not answer from \
                         the test network 2026-10-08",
            },
            // EUW is hosted in Amsterdam (Riot 2014 'Amsterdam datacenter' post).
            VALVE_AMS,
        ],
    },
    Row {
        id: "apex",
        game: "Apex Legends",
        publisher: "EA",
        region: "Frankfurt",
        // Hosting unpublished today (2019: Multiplay + Google Cloud); Frankfurt stand-in.
        targets: &AWS_FRA,
    },
    Row {
        id: "dota2",
        game: "Dota 2",
        publisher: "Valve",
        region: "Stockholm",
        targets: &[
            T {
                host: "162.254.198.41",
                probes: ICMP,
                stand_in: false,
                source: "Valve SDR relay 'sto' (Stockholm-Kista), GetSDRConfig appid=570",
            },
            T {
                host: "155.133.252.37",
                probes: ICMP,
                stand_in: false,
                source: "Valve SDR relay 'sto2' (Stockholm-Bromma), GetSDRConfig appid=570",
            },
        ],
    },
];

pub fn eu() -> Vec<GameServer> {
    ROWS.iter()
        .map(|r| GameServer {
            id: r.id.into(),
            game: r.game.into(),
            publisher: r.publisher.into(),
            region: r.region.into(),
            targets: r
                .targets
                .iter()
                .map(|t| Target {
                    host: t.host.into(),
                    probes: t.probes.to_vec(),
                    stand_in: t.stand_in,
                    source: t.source.into(),
                })
                .collect(),
        })
        .collect()
}
