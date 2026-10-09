# cities.txt - the World clock's place list

Built by `cargo run -p bu-timers --example timers-gencities -- <dir>` from three public files placed in `<dir>`:

* GeoNames `cities15000.zip` (cities of 15 000 people or more) and `countryInfo.txt` - https://download.geonames.org/export/dump/ -
  licence **CC BY 4.0** (credit: GeoNames, geonames.org).
* Unicode CLDR `common/supplemental/windowsZones.xml` (IANA zone -> Windows zone key) - Unicode licence.

Layout: see the head of `examples/gen_cities.rs`. Pulled 2026-10-09: 34 156 rows -> 33 437 places (same name + country + zone
dropped), 127 Windows zones, 244 countries, 620 183 bytes. The app only reads the text while the "Add a place" box searches.
