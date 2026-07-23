// Curated starter packs — ready-made channel sets an operator can install into
// their Memories with one click (or offered at first run). Bundled (works offline,
// no external dependency); a hosted refresh can layer on later. North America first.
//
// The data here is frequency *conventions* (calling channels, digital watering
// holes, POTA activity) plus a few well-known HF nets. Net schedules are UTC and
// approximate — reminders are opt-in and the operator can adjust the time.

import {
  addGroup,
  addMemory,
  coerceMemory,
  memoryKey,
  updateMemory,
  type MemoriesBank,
  type Memory,
} from './memories'

export interface PackMemory extends Partial<Memory> {
  name: string
  rxMhz: number
  mode: string
}

export interface Pack {
  id: string
  name: string
  /** One-line description shown on the pack card. */
  description: string
  /** Region tag, e.g. "North America". */
  region: string
  memories: PackMemory[]
}

// --- North America starter packs -------------------------------------------

const CALLING: Pack = {
  id: 'na-calling',
  name: 'VHF/UHF Calling & Simplex',
  description: 'National FM & SSB calling channels — where to find and make contacts (unchanged canonical home for these freqs).',
  region: 'North America',
  memories: [
    { name: '2 m FM Calling', rxMhz: 146.52, mode: 'FM', kind: 'simplex', notes: '2 m FM national simplex calling; also the go-to grid-down/EmComm hailing freq. R1 (EU) uses 145.500.' },
    { name: '70 cm FM Calling', rxMhz: 446, mode: 'FM', kind: 'simplex', notes: '70 cm FM national simplex calling. R1 uses 433.500.' },
    { name: '6 m FM Calling', rxMhz: 52.525, mode: 'FM', kind: 'simplex', notes: '6 m \'magic band\' FM simplex calling. R1 uses 51.510.' },
    { name: '1.25 m FM Calling', rxMhz: 223.5, mode: 'FM', kind: 'simplex', notes: '222 MHz FM simplex calling (US/Canada allocation).' },
    { name: '23 cm FM Calling', rxMhz: 1294.5, mode: 'FM', kind: 'simplex', notes: '1296 MHz FM simplex calling; regional band plans vary.' },
    { name: '10 m FM Calling', rxMhz: 29.6, mode: 'FM', kind: 'simplex', notes: '10 m FM simplex calling (FM segment 29.5–29.7).' },
    { name: '6 m SSB Calling', rxMhz: 50.125, mode: 'USB', kind: 'calling', notes: '6 m weak-signal SSB calling; make contact then QSY up. R1 uses 50.150.' },
    { name: '2 m SSB Calling', rxMhz: 144.2, mode: 'USB', kind: 'calling', notes: '2 m weak-signal/horizontal SSB calling; primary VHF-contest/rover call. R1 uses 144.300.' },
    { name: '70 cm SSB Calling', rxMhz: 432.1, mode: 'USB', kind: 'calling', notes: '70 cm weak-signal SSB/CW calling. R1 uses 432.200.' },
    { name: '23 cm SSB Calling', rxMhz: 1296.1, mode: 'USB', kind: 'calling', notes: '1296 MHz weak-signal SSB/CW calling. R1 centre of activity 1296.200.' },
  ],
}

const DIGITAL: Pack = {
  id: 'na-digital',
  name: 'HF FT8 & FT4',
  description: 'Full WSJT-X default FT8 and FT4 dial frequencies across the HF bands + 6 m.',
  region: 'Worldwide',
  memories: [
    { name: 'FT8 80 m', rxMhz: 3.573, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default USB dial.' },
    { name: 'FT8 60 m', rxMhz: 5.357, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default; 60 m is channelized/100 W ERP in the US.' },
    { name: 'FT8 40 m', rxMhz: 7.074, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default. Busiest evening/NVIS FT8 band.' },
    { name: 'FT8 30 m', rxMhz: 10.136, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default; reliable low-noise WARC band.' },
    { name: 'FT8 20 m', rxMhz: 14.074, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default. The #1 HF/POTA digital hunting freq.' },
    { name: 'FT8 17 m', rxMhz: 18.1, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default.' },
    { name: 'FT8 15 m', rxMhz: 21.074, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default; daytime DX.' },
    { name: 'FT8 12 m', rxMhz: 24.915, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default (WARC).' },
    { name: 'FT8 10 m', rxMhz: 28.074, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default; Es/solar-max.' },
    { name: 'FT8 6 m', rxMhz: 50.313, mode: 'FT8', kind: 'digital', notes: 'WSJT-X default \'magic band\' — the most-watched channel during Es/F2. EU adds 50.323 as an overflow dial.' },
    { name: 'FT4 80 m', rxMhz: 3.575, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default (distinct from FT8 3.573).' },
    { name: 'FT4 40 m', rxMhz: 7.0475, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default (R2). R1/R3 differ.' },
    { name: 'FT4 30 m', rxMhz: 10.14, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default.' },
    { name: 'FT4 20 m', rxMhz: 14.08, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default. Overlaps JS8/RTTY real estate — watch passband.' },
    { name: 'FT4 17 m', rxMhz: 18.104, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default (same dial as JS8 17 m).' },
    { name: 'FT4 15 m', rxMhz: 21.14, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default.' },
    { name: 'FT4 10 m', rxMhz: 28.18, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default.' },
    { name: 'FT4 6 m', rxMhz: 50.318, mode: 'FT4', kind: 'digital', notes: 'WSJT-X default; contest-rate Es runs.' },
  ],
}

const DIGITAL_MODES: Pack = {
  id: 'na-digital-modes',
  name: 'Digital Watering Holes (JS8, PSK31, RTTY, SSTV, VarAC)',
  description: 'HF digital activity centers beyond FT8/FT4. Conventions, not channels — tune the waterfall.',
  region: 'Worldwide',
  memories: [
    { name: 'JS8Call 40 m', rxMhz: 7.078, mode: 'JS8', kind: 'digital', notes: 'USB dial. Busiest JS8Call watering hole (evenings/NVIS). Full offsets: 1.842/3.578/7.078/10.130/14.078/18.104/21.078/24.922/28.078/50.318.' },
    { name: 'JS8Call 20 m', rxMhz: 14.078, mode: 'JS8', kind: 'digital', notes: 'USB dial. Main daytime JS8Call freq; sits between FT8 (14.074) and FT4 (14.080).' },
    { name: 'JS8Call 30 m', rxMhz: 10.13, mode: 'JS8', kind: 'digital', notes: 'USB dial. Low-noise WARC option; data-only worldwide.' },
    { name: 'JS8Call 80 m', rxMhz: 3.578, mode: 'JS8', kind: 'digital', notes: 'USB dial. Regional/NVIS after dark.' },
    { name: 'PSK31 20 m', rxMhz: 14.07, mode: 'PSK31', kind: 'digital', notes: 'USB dial — classic PSK31/PSK63 hangout; signals stack 14.070–14.072.' },
    { name: 'PSK31 40 m', rxMhz: 7.07, mode: 'PSK31', kind: 'digital', notes: 'USB dial (R2/US). R1 PSK31 sits 7.035–7.040.' },
    { name: 'PSK31 80 m', rxMhz: 3.58, mode: 'PSK31', kind: 'digital', notes: 'USB dial. Regional/NVIS after dark.' },
    { name: 'PSK31 30 m', rxMhz: 10.142, mode: 'PSK31', kind: 'digital', notes: 'USB dial. Shares narrowband real estate — listen first.' },
    { name: 'PSK31 15 m', rxMhz: 21.07, mode: 'PSK31', kind: 'digital', notes: 'USB dial. Daytime PSK31/PSK63 DX.' },
    { name: 'RTTY 20 m', rxMhz: 14.08, mode: 'RTTY', kind: 'digital', notes: '45.45 baud/170 Hz shift, LSB convention. Everyday ~14.080–14.099; contests fill 14.070–14.099.' },
    { name: 'RTTY 40 m', rxMhz: 7.04, mode: 'RTTY', kind: 'digital', notes: 'DX RTTY ~7.040; US/contest RTTY 7.025–7.080.' },
    { name: 'RTTY 15 m', rxMhz: 21.08, mode: 'RTTY', kind: 'digital', notes: 'Daytime RTTY DX ~21.080; contest spread 21.070–21.099.' },
    { name: 'SSTV 20 m', rxMhz: 14.23, mode: 'USB', kind: 'digital', notes: 'Worldwide SSTV calling (Scottie/Martin/PD). Wide (~2.5 kHz) — needs guard space.' },
    { name: 'SSTV 40 m', rxMhz: 7.171, mode: 'USB', kind: 'digital', notes: 'US SSTV evenings. R1 commonly near 7.165.' },
    { name: 'SSTV 10 m', rxMhz: 28.68, mode: 'USB', kind: 'digital', notes: 'Active during Es/solar-peak openings; easy 10 m SSTV DX.' },
    { name: 'VarAC 20 m', rxMhz: 14.105, mode: 'DIG', kind: 'digital', notes: 'VARA HF USB dial — main VarAC calling (keyboard chat + store-and-forward). Full set 1.995/3.595/7.105/10.133/14.105/18.107/21.105/24.927/28.105.' },
    { name: 'VarAC 40 m', rxMhz: 7.105, mode: 'DIG', kind: 'digital', notes: 'VARA HF USB dial. Primary evening/NVIS VarAC. VARA is wide — monitor first.' },
    { name: 'VarAC 80 m', rxMhz: 3.595, mode: 'DIG', kind: 'digital', notes: 'VARA HF USB dial. Regional after-dark VarAC.' },
    { name: 'Olivia 20 m', rxMhz: 14.073, mode: 'Olivia', kind: 'digital', notes: '~14.073 USB dial, center 1500 Hz, 8/250 or 16/500. Contestia shares. Convention, not a channel — listen ±500 Hz.' },
    { name: 'Olivia 40 m', rxMhz: 7.073, mode: 'Olivia', kind: 'digital', notes: '~7.073 USB dial, center 1500 Hz. Informal watering hole; R1 activity tends lower.' },
  ],
}

const CW_QRP: Pack = {
  id: 'na-cw-qrp',
  name: 'CW & QRP Watering Holes',
  description: 'QRP CW calling frequencies plus SKCC/FISTS/CWops club centers of activity.',
  region: 'Worldwide',
  memories: [
    { name: '160 m QRP CW', rxMhz: 1.81, mode: 'CW', kind: 'calling', notes: 'Top-band QRP CW near the low CW/DX end; winter nights, a real challenge.' },
    { name: '80 m QRP CW', rxMhz: 3.56, mode: 'CW', kind: 'calling', notes: 'International QRP CW calling; regional/NVIS after dark.' },
    { name: '40 m QRP CW', rxMhz: 7.03, mode: 'CW', kind: 'calling', notes: 'International QRP CW calling & 40 m CW DX calling. R2 ops sometimes use 7.040 to sidestep R1.' },
    { name: '30 m QRP CW', rxMhz: 10.106, mode: 'CW', kind: 'calling', notes: '30 m QRP CW center; WARC CW/data only, digital lives 10.130+.' },
    { name: '20 m QRP CW', rxMhz: 14.06, mode: 'CW', kind: 'calling', notes: 'The busiest QRP CW hangout worldwide.' },
    { name: '17 m QRP CW', rxMhz: 18.086, mode: 'CW', kind: 'calling', notes: '17 m QRP CW center (WARC, no contests).' },
    { name: '15 m QRP CW', rxMhz: 21.06, mode: 'CW', kind: 'calling', notes: '15 m QRP CW calling; productive on daytime solar openings.' },
    { name: '12 m QRP CW', rxMhz: 24.906, mode: 'CW', kind: 'calling', notes: '12 m QRP CW calling (WARC); sparse but rewarding at solar peak.' },
    { name: '10 m QRP CW', rxMhz: 28.06, mode: 'CW', kind: 'calling', notes: '10 m QRP CW calling; big signals on little power in Es/F2 (QRP crowd 28.058–28.060).' },
    { name: 'SKCC 80 m', rxMhz: 3.55, mode: 'CW', kind: 'calling', notes: 'Straight Key Century Club center of activity (mechanical keys). Evening regional.' },
    { name: 'SKCC 40 m', rxMhz: 7.055, mode: 'CW', kind: 'calling', notes: 'SKCC 40 m center; use the SKCC Sked Page to arrange contacts. Also ARRL Straight Key Night (Jan 1) anchor.' },
    { name: 'SKCC 30 m', rxMhz: 10.12, mode: 'CW', kind: 'calling', notes: 'SKCC 30 m center (WARC); quiet band for relaxed straight-key QSOs.' },
    { name: 'SKCC 20 m', rxMhz: 14.05, mode: 'CW', kind: 'calling', notes: 'SKCC 20 m center; daytime straight-key ragchews.' },
    { name: 'FISTS 80 m', rxMhz: 3.558, mode: 'CW', kind: 'calling', notes: 'FISTS CW Club calling (.558). Newcomer/Elmer friendly, code-speed-matching.' },
    { name: 'FISTS 40 m', rxMhz: 7.058, mode: 'CW', kind: 'calling', notes: 'FISTS calling in R2 (7.028 used all-regions). Slower CW welcomed.' },
    { name: 'FISTS 20 m', rxMhz: 14.058, mode: 'CW', kind: 'calling', notes: 'FISTS 20 m calling; look for members on/near .058.' },
    { name: 'CWops 40 m', rxMhz: 7.028, mode: 'CW', kind: 'calling', notes: 'CWops center of activity. Full set 1.818/3.528/7.028/10.118/14.028/18.078/21.028/24.908/28.028. Mini-CWT hour tests Wed 1300/1900Z & Thu 0300/0700Z near these — verify times at cwops.org.' },
    { name: 'CWops 20 m', rxMhz: 14.028, mode: 'CW', kind: 'calling', notes: 'CWops 20 m center of activity; representative CWT run spot.' },
  ],
}

const EMCOMM: Pack = {
  id: 'na-emcomm',
  name: 'Emergency & EmComm',
  description: 'Hurricane/emergency HF nets, IARU emergency centers, 60 m interop, plus GMRS/marine/aircraft reference. SKYWARN has NO national freq — it runs on your local NWS-affiliated repeaters.',
  region: 'North America',
  memories: [
    { name: 'Hurricane Watch Net (Day)', rxMhz: 14.325, mode: 'USB', kind: 'emcomm', notes: 'Activation-only: stands up when a hurricane is ~300 mi from landfall. Surface reports relayed to WX4NHC at NHC Miami. Switches to 7.268 LSB at night. Monitor during storms, not a daily net.' },
    { name: 'Hurricane Watch Net (Night)', rxMhz: 7.268, mode: 'LSB', kind: 'emcomm', notes: 'Night/short-skip HWN during activations — same net as 14.325. (Waterway Radio & Cruising Club shares this dial daytime ~1145 UTC; HWN has priority during storms.)' },
    { name: 'Maritime Mobile Service Net', rxMhz: 14.3, mode: 'USB', kind: 'hfnet', notes: '20 m maritime safety/health-welfare net, daily ~1600–0200 UTC. 14.300 is the IARU global emergency Centre of Activity — the most-monitored HF emergency SSB freq. Shares dial with Intercon & Pacific Seafarers nets.' },
    { name: 'IARU Emergency COA 17 m', rxMhz: 18.16, mode: 'USB', kind: 'emcomm', notes: 'IARU global emergency Centre of Activity (all 3 regions). Keep clear during declared emergencies.' },
    { name: 'IARU Emergency COA 15 m', rxMhz: 21.36, mode: 'USB', kind: 'emcomm', notes: 'IARU global emergency Centre of Activity (all 3 regions).' },
    { name: '60 m Interop Channel 1', rxMhz: 5.3305, mode: 'USB', kind: 'emcomm', notes: 'US 60 m channelized ham/federal (SHARES) interop + daytime NVIS. Full set: 5330.5/5346.5/5357.0/5371.5/5403.5 kHz USB, 100 W ERP.' },
    { name: 'GMRS/FRS Channel 1', rxMhz: 462.5625, mode: 'FM', kind: 'simplex', notes: 'Most common consumer bubble-pack channel; de-facto neighborhood calling in a grid-down scenario. Simplex, monitor with CTCSS off.' },
    { name: 'GMRS/FRS Channel 3 (prepper)', rxMhz: 462.6125, mode: 'FM', kind: 'simplex', notes: 'Widely promoted preparedness monitoring/calling channel. Unofficial convention — no FCC designation, adoption varies.' },
    { name: 'GMRS Ch 20 Travelers Assistance', rxMhz: 462.675, mode: 'FM', kind: 'emcomm', notes: 'Traditional GMRS emergency/travelers channel with 141.3 Hz CTCSS (\'20/141\'), from the old REACT convention. Still monitored by some clubs. Convention, not FCC mandate.' },
    { name: 'Marine VHF Channel 16', rxMhz: 156.8, mode: 'FM', kind: 'reference', notes: 'International maritime distress/safety/calling, monitored by USCG. Non-amateur — RX/awareness; transmit only if licensed/appropriate.' },
    { name: 'Aircraft Emergency \'Guard\'', rxMhz: 121.5, mode: 'AM', kind: 'reference', notes: 'International aeronautical emergency (VHF AM). Cross-service RX/awareness only; not an amateur frequency.' },
    { name: 'SATERN International Net', rxMhz: 14.265, mode: 'USB', kind: 'emcomm', notes: 'Salvation Army emergency net — historically daily 1500–1600 UTC on 14.265, but the regular net is discontinued/dormant and satern.org lapsed. Treat as activation-only; verify before relying.' },
  ],
}

const HF_NETS: Pack = {
  id: 'na-hfnets',
  name: 'HF Traffic & Ragchew Nets',
  description: 'Daily awards, county-hunting and traffic nets (US/R2). Times are UTC and approximate — verify and set your own reminder.',
  region: 'North America',
  memories: [
    { name: 'OMISS 20 m SSB Net', rxMhz: 14.29, mode: 'USB', kind: 'hfnet', notes: 'One-Man-Intl-SSB worked-all-states awards net, daily ~1830 UTC; pairs stations for state exchanges.' },
    { name: 'OMISS 40 m SSB Net', rxMhz: 7.194, mode: 'LSB', kind: 'hfnet', notes: 'OMISS evening WAS net ~0100 UTC (late ~0330 UTC Sat/Sun).' },
    { name: 'OMISS 80 m SSB Net', rxMhz: 3.825, mode: 'LSB', kind: 'hfnet', notes: 'OMISS 80 m net ~0300 UTC (late ~0500 UTC Sat/Sun).' },
    { name: 'County Hunters 20 m SSB', rxMhz: 14.336, mode: 'USB', kind: 'hfnet', notes: 'Primary mobile county-line net (USACA award); mobiles announce county and QSY on request.' },
    { name: 'County Hunters 20 m CW', rxMhz: 14.0565, mode: 'CW', kind: 'calling', notes: '20 m CW county-hunting calling (14.056.5); 40 m counterpart 7.056.5.' },
    { name: 'County Hunters 40 m SSB', rxMhz: 7.188, mode: 'LSB', kind: 'hfnet', notes: '40 m mobile county-hunting SSB calling; mobiles QSY off on request.' },
    { name: '7290 Traffic Net', rxMhz: 7.29, mode: 'LSB', kind: 'hfnet', notes: 'Long-running informal 40 m traffic/ragchew net (TX/OK roots); meets daily, multiple sessions.' },
    { name: '3905 Century Club 80 m', rxMhz: 3.905, mode: 'LSB', kind: 'hfnet', notes: 'Namesake 80 m SSB WAS/awards net, structured swap-and-confirm, evenings. Verify schedule on 3905ccn.com.' },
    { name: '3905 Century Club 40 m', rxMhz: 7.233, mode: 'LSB', kind: 'hfnet', notes: '3905CCN 40 m SSB awards net; logs contacts toward club WAS. Verify on 3905ccn.com.' },
    { name: '10-10 International 10 m', rxMhz: 28.38, mode: 'USB', kind: 'hfnet', notes: 'Ten-Ten net/calling activity ~28.380–28.400 USB; members exchange 10-10 numbers toward awards. Active when 10 m is open.' },
  ],
}

const VHF_WEAK: Pack = {
  id: 'na-vhf-weak',
  name: 'VHF+ Weak-Signal & Digital',
  description: '6 m/2 m/70 cm/23 cm CW, DX, FT8, meteor-scatter, EME and APRS. (SSB calling lives in the VHF/UHF Calling pack.)',
  region: 'North America',
  memories: [
    { name: '6 m CW Calling', rxMhz: 50.09, mode: 'CW', kind: 'calling', notes: '6 m CW calling (activity 50.080–50.100). Beacons live below ~50.080.' },
    { name: '6 m DX Calling', rxMhz: 50.11, mode: 'USB', kind: 'calling', notes: 'Intercontinental DX window 50.100–50.125; keep domestic QSOs off this.' },
    { name: '2 m FT8', rxMhz: 144.174, mode: 'FT8', kind: 'digital', notes: 'Main 2 m weak-signal digital; tropo/Es/EME ops monitor here.' },
    { name: '70 cm FT8', rxMhz: 432.174, mode: 'FT8', kind: 'digital', notes: '70 cm FT8 dial; standard for tropo openings, lower activity than 2 m.' },
    { name: '23 cm FT8', rxMhz: 1296.174, mode: 'FT8', kind: 'digital', notes: '1296 MHz FT8; sparse — coordinate skeds via ON4KST chat.' },
    { name: '6 m Meteor Scatter (MSK144)', rxMhz: 50.26, mode: 'MSK144', kind: 'digital', notes: '6 m MSK144 meteor-scatter calling (USB dial); showers + daytime pings.' },
    { name: '2 m Meteor Scatter (MSK144)', rxMhz: 144.15, mode: 'MSK144', kind: 'digital', notes: 'NA 2 m MSK144 calling (USB dial). R1 uses 144.360. Peaks Perseids/Geminids.' },
    { name: '2 m EME (Q65/CW)', rxMhz: 144.116, mode: 'Q65', kind: 'digital', notes: '2 m moonbounce digital (Q65 has replaced JT65); random CW EME 144.100–144.120. Coordinate on ON4KST/N0UK logger.' },
    { name: '70 cm EME (Q65/CW)', rxMhz: 432.065, mode: 'Q65', kind: 'digital', notes: '70 cm moonbounce digital ~432.065; CW EME clusters 432.010–432.070. Full-moon weekends most active.' },
    { name: 'APRS (North America)', rxMhz: 144.39, mode: 'FM', kind: 'digital', notes: '1200-baud APRS network for US/Canada/Mexico. R1 144.800, Japan 144.640/660, VK/ZL 145.175. Igates/digipeaters + position beacons.' },
  ],
}

const SATS: Pack = {
  id: 'na-sats',
  name: 'Amateur Satellites',
  description: 'Active FM easy-sats, linear transponders, ISS and digipeaters. VOLATILE — reconfirm each bird at amsat.org/status before a pass.',
  region: 'Worldwide',
  memories: [
    { name: 'SO-50 (SaudiSat-1C) FM', rxMhz: 436.795, mode: 'FM', kind: 'satellite', notes: 'Most popular FM easy-sat. Uplink 145.850 FM + 67.0 Hz PL to talk; ARM the 10-min onboard timer first with a 2-sec carrier + 74.4 Hz tone. Downlink shown Doppler-corrected. Operational.' },
    { name: 'AO-91 (RadFxSat/Fox-1B) FM', rxMhz: 145.96, mode: 'FM', kind: 'satellite', notes: 'U/v FM. Uplink 435.250 + 67.0 Hz PL. Operational but battery-limited — avoid eclipse passes.' },
    { name: 'PO-101 (Diwata-2) FM', rxMhz: 145.9, mode: 'FM', kind: 'satellite', notes: 'U/v FM. Uplink 437.500 + 141.3 Hz PL. Transponder ON only in scheduled activation windows (AMSAT announces); off otherwise.' },
    { name: 'ISS FM Voice Repeater', rxMhz: 437.8, mode: 'FM', kind: 'satellite', notes: 'Cross-band FM repeater (Kenwood D710GA, Columbus). Uplink 145.990 + 67.0 Hz PL. Off during ARISS school contacts, SSTV events or crew ops. Operational.' },
    { name: 'ISS APRS Digipeater (NA1SS)', rxMhz: 145.825, mode: 'DIG', kind: 'digital', notes: '145.825 simplex up+down, 1200-baud AFSK APRS, alias ARISS. Digipeats packets/messaging. Usually on when the voice repeater is not.' },
    { name: 'ISS Voice / ARISS Downlink', rxMhz: 145.8, mode: 'FM', kind: 'satellite', notes: 'Crew voice + ARISS school QSOs downlink 145.800 worldwide. Uplink 144.490 (R2/3) or 145.200 (R1). SSTV events also use 145.800 FM (PD120). Listen-only for scheduled contacts.' },
    { name: 'RS-44 (DOSAAF-85) Linear', rxMhz: 435.64, mode: 'USB/LSB/CW', kind: 'satellite', notes: 'V/u inverting linear. Uplink 145.935–145.995 LSB, downlink 435.610–435.670 USB (center ~435.640). High orbit = long passes, big footprint. Operational.' },
    { name: 'FO-29 (JAS-2) Linear', rxMhz: 435.85, mode: 'USB/LSB/CW', kind: 'satellite', notes: 'V/u inverting. Uplink 145.900–146.000 LSB, downlink 435.800–435.900 USB. Aging, schedule/sunlight-limited — verify status before use.' },
    { name: 'AO-73 (FUNcube-1) Linear', rxMhz: 145.97, mode: 'USB/LSB/CW', kind: 'satellite', notes: 'U/v inverting. Uplink 435.130–435.150 LSB, downlink 145.960–145.980 USB; BPSK telemetry ~145.935. Runs transponder or telemetry per schedule.' },
    { name: 'IO-117 (GreenCube) Digipeater', rxMhz: 435.31, mode: 'DIG', kind: 'digital', notes: 'UHF store-and-forward digipeater (1200/9600 bps). MEO ~5900 km = huge footprint, prized for sat DX/grid chasing. Confirm status before a pass.' },
    { name: 'TEVEL-2 Constellation FM', rxMhz: 436.4, mode: 'FM', kind: 'satellite', notes: 'Israeli CubeSat swarm, U/v FM. Uplink 145.970; 9600 BPSK beacon shares the downlink. Units toggle transponder on schedule; multiple active.' },
    { name: 'QO-100 (Es\'hail-2) NB', rxMhz: 10489.75, mode: 'USB/CW/DIG', kind: 'satellite', notes: 'Geostationary at 26E. NB uplink 2400.000–2400.500 (S-band), downlink 10489.500–10490.000 (X-band). Always-on but NOT visible from North America (EU/Africa/Mideast/W.Asia). Save for reference/travel.' },
  ],
}

const POTA_SOTA: Pack = {
  id: 'na-pota-sota',
  name: 'POTA / SOTA / WWFF',
  description: 'Parks/Summits/Flora-Fauna activity centers — CW, SSB and FT8. SSB freqs are social conventions; activators roam ±20 kHz.',
  region: 'North America',
  memories: [
    { name: 'POTA 40 m FT8', rxMhz: 7.074, mode: 'FT8', kind: 'pota', notes: 'WSJT-X default; workhorse park-to-park at night/regional.' },
    { name: 'POTA 20 m FT8', rxMhz: 14.074, mode: 'FT8', kind: 'pota', notes: 'WSJT-X default; the #1 POTA/SOTA digital hunting freq.' },
    { name: 'POTA/SOTA 40 m CW', rxMhz: 7.032, mode: 'CW', kind: 'pota', notes: 'POTA/QRP CW cluster 7.030–7.040; SOTA CW also here. 7.058 common alternate.' },
    { name: 'POTA/SOTA 30 m CW', rxMhz: 10.116, mode: 'CW', kind: 'pota', notes: '30 m CW/digital only; POTA clusters 10.110–10.120, SOTA calling ~10.118. Low-QRM daytime band.' },
    { name: 'POTA 20 m CW', rxMhz: 14.032, mode: 'CW', kind: 'pota', notes: 'POTA CW cluster 14.030–14.060; 14.060 is the QRP calling freq many activators favor.' },
    { name: 'POTA 15 m CW', rxMhz: 21.032, mode: 'CW', kind: 'pota', notes: 'POTA 15 m CW activity when band open.' },
    { name: 'POTA 80 m SSB', rxMhz: 3.885, mode: 'LSB', kind: 'pota', notes: 'Evening/close-in POTA SSB watering hole (General phone segment). Note: earlier app builds used 3.985 — 3.885 is the more standard POTA spot.' },
    { name: 'POTA/SOTA 40 m SSB', rxMhz: 7.185, mode: 'LSB', kind: 'pota', notes: 'Primary POTA 40 m SSB gathering (7.180–7.200); 7.285 common alternate.' },
    { name: 'POTA/SOTA 20 m SSB', rxMhz: 14.285, mode: 'USB', kind: 'pota', notes: 'Busiest POTA/SOTA SSB watering hole; hunters park here to find activators. R1 SOTA SSB often 14.310/14.345.' },
    { name: 'POTA 15 m SSB', rxMhz: 21.285, mode: 'USB', kind: 'pota', notes: 'POTA 15 m SSB when band open; 21.385 common alternate.' },
    { name: 'POTA 10 m SSB', rxMhz: 28.45, mode: 'USB', kind: 'pota', notes: 'POTA 10 m SSB during solar-active/Es openings.' },
    { name: 'SOTA 2 m FM', rxMhz: 146.52, mode: 'FM', kind: 'pota', notes: 'NA national simplex — the go-to for VHF-only summit activations. R1 SOTA uses 145.500.' },
    { name: 'SOTA 20 m CW', rxMhz: 14.062, mode: 'CW', kind: 'pota', notes: 'Long-standing international SOTA CW calling; spot via SOTAwatch/SOTA Spotter.' },
    { name: 'SOTA 2 m SSB', rxMhz: 144.2, mode: 'USB', kind: 'pota', notes: 'US 2 m SSB calling for weak-signal summit-to-summit. R1 uses 144.300.' },
    { name: 'WWFF 20 m SSB', rxMhz: 14.244, mode: 'USB', kind: 'calling', notes: 'WWFF \'x44\' convention so hunters find flora/fauna activators.' },
    { name: 'WWFF 40 m SSB', rxMhz: 7.144, mode: 'LSB', kind: 'calling', notes: 'WWFF 40 m \'x44\' calling (DX phone segment — 7.144 OK for US General).' },
    { name: 'WWFF 20 m CW', rxMhz: 14.044, mode: 'CW', kind: 'calling', notes: 'WWFF CW \'x44\' convention; also 7.024/10.124/21.044/28.044.' },
  ],
}

const DX_CONTEST: Pack = {
  id: 'na-dx-contest',
  name: 'DX & Contest',
  description: 'Per-band DX windows (CW/SSB), IOTA calling, DXpedition FT8 Fox/Hound set and contest run sub-bands. On the day, trust the DXpedition\'s own announced QRG + offset.',
  region: 'Worldwide',
  memories: [
    { name: '160 m Top Band DX', rxMhz: 1.8265, mode: 'CW', kind: 'calling', notes: '1.822–1.830 CW window; DX TX ~1.826.5 and listens up. Transatlantic split common (R1 starts 1.810). Gray-line/winter-night.' },
    { name: '80 m CW DX', rxMhz: 3.523, mode: 'CW', kind: 'calling', notes: '3.505–3.525, DXpeditions listen up. Best around local sunrise/sunset.' },
    { name: '80 m SSB DX', rxMhz: 3.795, mode: 'LSB', kind: 'calling', notes: '3.790–3.800 phone DX window; cross-band transatlantic split common (R1 phone from 3.600).' },
    { name: '40 m CW DX', rxMhz: 7.023, mode: 'CW', kind: 'calling', notes: 'DXpedition CW 7.020–7.026, listening up. 7.023/7.026 perennial spots.' },
    { name: '40 m SSB DX', rxMhz: 7.16, mode: 'LSB', kind: 'calling', notes: 'Region-messy: DX usually TX 7.150–7.170 and LISTENS UP 7.175–7.200 for NA. Always split — check announced QRG.' },
    { name: '30 m CW DX', rxMhz: 10.105, mode: 'CW', kind: 'calling', notes: 'DXpeditions 10.103–10.115 CW. WARC = no contests, split discouraged — most run simplex.' },
    { name: '20 m CW DX', rxMhz: 14.023, mode: 'CW', kind: 'calling', notes: '14.020–14.025 DX segment; listen up 1–5 kHz. Band opens here first.' },
    { name: '20 m SSB DX', rxMhz: 14.195, mode: 'USB', kind: 'calling', notes: '14.190–14.200 DX window — the single most-used DXpedition SSB freq worldwide; they listen up 5–15 kHz. NEVER call on the DX\'s own freq.' },
    { name: '17 m CW DX', rxMhz: 18.075, mode: 'CW', kind: 'calling', notes: 'DXpedition CW 18.075–18.079, listening up. WARC = contest-free.' },
    { name: '17 m SSB DX', rxMhz: 18.145, mode: 'USB', kind: 'calling', notes: 'WARC phone DX slot ~18.130–18.145; calm band for DX.' },
    { name: '15 m CW DX', rxMhz: 21.023, mode: 'CW', kind: 'calling', notes: 'DXpedition CW 21.020–21.025, listening up.' },
    { name: '15 m SSB DX', rxMhz: 21.295, mode: 'USB', kind: 'calling', notes: '21.290–21.300 DX phone slot; strong daytime band at solar max. Listen up.' },
    { name: '12 m CW DX', rxMhz: 24.895, mode: 'CW', kind: 'calling', notes: 'DXpedition CW 24.895–24.900 (WARC, no contests).' },
    { name: '12 m SSB DX', rxMhz: 24.945, mode: 'USB', kind: 'calling', notes: 'WARC phone DX ~24.945–24.960; excellent at high solar flux.' },
    { name: '10 m CW DX', rxMhz: 28.023, mode: 'CW', kind: 'calling', notes: 'DXpedition CW 28.020–28.025. Watch beacons 28.190–28.300 to sense openings.' },
    { name: '10 m SSB DX', rxMhz: 28.495, mode: 'USB', kind: 'calling', notes: '10 m DXpedition phone 28.490–28.500; big daytime band when the sun is active.' },
    { name: 'IOTA SSB Calling', rxMhz: 14.26, mode: 'USB', kind: 'calling', notes: 'Worldwide island-activity SSB calling; island DXpeditions work here or announce QSY. Also 7.055/7.160/21.260/28.460/28.560.' },
    { name: 'IOTA CW Calling', rxMhz: 14.04, mode: 'CW', kind: 'calling', notes: 'Long-standing IOTA CW activity; band-specific near 3.530/7.030/21.040/28.040. RSGB IOTA Contest = last full weekend of July.' },
    { name: 'DXpedition FT8 20 m (Fox/Hound)', rxMhz: 14.09, mode: 'FT8', kind: 'digital', notes: 'SEPARATE from 14.074. WSJT-X Fox/Hound: Fox (DX) TX <1000 Hz, Hounds TX >1000 Hz and auto-QSY — enable Hound mode. Full F/H set 1.908/3.567/7.056/10.131/14.090/18.095/21.091/24.911/28.091.' },
    { name: 'DXpedition FT8 40 m (Fox/Hound)', rxMhz: 7.056, mode: 'FT8', kind: 'digital', notes: 'Standard 40 m DXpedition F/H freq (not the 7.074 general FT8 QRG). Hound mode required.' },
    { name: 'CW Contest Sub-bands', rxMhz: 14.035, mode: 'CW', kind: 'reference', notes: 'Where to spin during CW contests (CQ WW/WPX, ARRL DX): 1.800–1.840/3.500–3.560/7.000–7.060/14.000–14.070/21.000–21.070/28.000–28.070. Fill from band edge up.' },
    { name: 'SSB Contest Sub-bands', rxMhz: 14.25, mode: 'USB', kind: 'reference', notes: 'Phone-contest runs (R2/US): 1.840–1.850/3.600–3.800/7.125–7.200/14.150–14.350/21.200–21.450/28.300–28.600. Avoid 14.100 (IBP beacons) + DX windows. R1 differs.' },
    { name: 'RTTY Contest Sub-bands', rxMhz: 14.085, mode: 'RTTY', kind: 'reference', notes: 'RTTY contest areas (CQ WW RTTY, ARRL RTTY Roundup): 3.570–3.600/7.025–7.070/14.070–14.099/21.070–21.099/28.070–28.099. Keep clear of FT8/FT4.' },
  ],
}

const REFERENCE: Pack = {
  id: 'na-reference',
  name: 'Reference: Time, Beacons & Utility Listening',
  description: 'Receive-only anchors: WWV/WWVH/CHU time & propagation, the NCDXF/IARU beacon set, plus US Coast Guard WEFAX and VOLMET.',
  region: 'Worldwide',
  memories: [
    { name: 'WWV/WWVH 5 MHz', rxMhz: 5, mode: 'AM', kind: 'reference', notes: 'Standard time/freq. WWV (Fort Collins CO, male) + WWVH (Kekaha HI, female). 24/7 tones + UTC voice. Best all-hours channel.' },
    { name: 'WWV/WWVH 10 MHz', rxMhz: 10, mode: 'AM', kind: 'reference', notes: 'Both stations. Propagation/geophysical alerts by voice: WWV :18, WWVH :45 (SFI, A/K, storms). Most reliable daytime channel.' },
    { name: 'WWV/WWVH 15 MHz', rxMhz: 15, mode: 'AM', kind: 'reference', notes: 'Both stations. Strong daytime/DX; good higher-band check.' },
    { name: 'WWV/WWVH 2.5 MHz', rxMhz: 2.5, mode: 'AM', kind: 'reference', notes: 'Both stations. Lowest channel — best local-night/regional coverage.' },
    { name: 'WWV 20 MHz', rxMhz: 20, mode: 'AM', kind: 'reference', notes: 'WWV only (no WWVH). Daytime high-band propagation indicator.' },
    { name: 'WWV 25 MHz', rxMhz: 25, mode: 'AM', kind: 'reference', notes: 'WWV only; experimental/intermittent, reduced power. 12 m-band propagation gauge when audible.' },
    { name: 'CHU Canada 7.850 MHz', rxMhz: 7.85, mode: 'USB', kind: 'reference', notes: 'NRC Canada time (Ottawa). USB voice EN/FR + FSK minute time-code burst. Best mid-band, day & night.' },
    { name: 'CHU Canada 3.330 MHz', rxMhz: 3.33, mode: 'USB', kind: 'reference', notes: 'CHU low channel, best at night/regional.' },
    { name: 'CHU Canada 14.670 MHz', rxMhz: 14.67, mode: 'USB', kind: 'reference', notes: 'CHU high channel, best daytime/DX.' },
    { name: 'WWVB 60 kHz Time Code', rxMhz: 0.06, mode: 'LF', kind: 'reference', notes: 'NIST 60 kHz LF carrier (Fort Collins). Drives radio-controlled \'atomic\' clocks — reference only, no audio.' },
    { name: 'NCDXF Beacon 20 m', rxMhz: 14.1, mode: 'CW', kind: 'reference', notes: '18-station worldwide propagation beacon net; callsign @22 wpm + four power-stepped dashes (100/10/1/0.1 W), 10 s per band, 3-min round-robin. 17 of 18 active (YV5B off).' },
    { name: 'NCDXF Beacon 17 m', rxMhz: 18.11, mode: 'CW', kind: 'reference', notes: '17 m slot of the NCDXF/IARU beacon network; same round-robin schedule.' },
    { name: 'NCDXF Beacon 15 m', rxMhz: 21.15, mode: 'CW', kind: 'reference', notes: '15 m slot of the NCDXF/IARU beacon network.' },
    { name: 'NCDXF Beacon 12 m', rxMhz: 24.93, mode: 'CW', kind: 'reference', notes: '12 m slot; useful marginal-band opening check.' },
    { name: 'NCDXF Beacon 10 m', rxMhz: 28.2, mode: 'CW', kind: 'reference', notes: '10 m slot — prime tool for spotting Es/F2 openings by which beacons you hear.' },
    { name: 'New York VOLMET 6.604 MHz', rxMhz: 6.604, mode: 'USB', kind: 'reference', notes: 'Continuous NAT aviation weather (METAR/TAF). Full set 3.485/6.604/10.051/13.270 kHz USB. Best night/all-hours.' },
    { name: 'New York VOLMET 13.270 MHz', rxMhz: 13.27, mode: 'USB', kind: 'reference', notes: 'Daytime high-band New York NAT VOLMET.' },
    { name: 'NMF Boston WEFAX', rxMhz: 9.11, mode: 'FAX/USB', kind: 'reference', notes: 'USCG Boston HF radiofax (weather charts). Assigned 4235/6340.5/9110/12750 kHz; USB dial = assigned − 1.9 kHz.' },
    { name: 'NMG New Orleans WEFAX', rxMhz: 8.5039, mode: 'FAX/USB', kind: 'reference', notes: 'USCG New Orleans radiofax incl. tropical-cyclone charts. Assigned 4317.9/8503.9/12789.9/17146.4 kHz; USB dial = assigned − 1.9 kHz.' },
    { name: 'NMC Point Reyes WEFAX', rxMhz: 8.682, mode: 'FAX/USB', kind: 'reference', notes: 'USCG Point Reyes CA radiofax. Assigned 4346/8682/12786/17151.2/22527 kHz; USB dial = assigned − 1.9 kHz.' },
    { name: 'NOJ Kodiak WEFAX', rxMhz: 8.459, mode: 'FAX/USB', kind: 'reference', notes: 'USCG Kodiak AK radiofax. Assigned 2054/4298/8459/12412.5 kHz; USB dial = assigned − 1.9 kHz.' },
    { name: 'KVM70 Honolulu WEFAX', rxMhz: 11.09, mode: 'FAX/USB', kind: 'reference', notes: 'NWS Honolulu Pacific radiofax. Assigned 9982.5/11090/16135 kHz; USB dial = assigned − 1.9 kHz.' },
  ],
}

export const STARTER_PACKS: Pack[] = [CALLING, DIGITAL, DIGITAL_MODES, CW_QRP, EMCOMM, HF_NETS, VHF_WEAK, SATS, POTA_SOTA, DX_CONTEST, REFERENCE]

/** The fields a pack is the authority on, as a patch against a row it still owns.
 * Everything absent from the pack entry is patched to `undefined` so a correction
 * that REMOVES a note/tone clears it rather than leaving the stale value behind.
 * User-owned state is not in here and survives: id, groups, favorite, lastUsedUtc —
 * and, for a net, the operator's own reminder prefs (the pack owns WHEN the net
 * meets; the operator owns whether they're reminded and how early). */
function packContentPatch(pm: PackMemory, existing: Memory): Partial<Memory> {
  const patch: Partial<Memory> = {
    name: pm.name,
    kind: pm.kind,
    rxMhz: pm.rxMhz,
    mode: pm.mode,
    offsetDir: pm.offsetDir,
    offsetMhz: pm.offsetMhz,
    txMhz: pm.txMhz,
    toneMode: pm.toneMode,
    ctcssEncHz: pm.ctcssEncHz,
    ctcssDecHz: pm.ctcssDecHz,
    dtcsCode: pm.dtcsCode,
    dtcsRxCode: pm.dtcsRxCode,
    dtcsPol: pm.dtcsPol,
    notes: pm.notes,
    callsign: pm.callsign,
    grid: pm.grid,
    skip: pm.skip,
    net: pm.net
      ? {
          ...pm.net,
          alertEnabled: existing.net?.alertEnabled ?? pm.net.alertEnabled,
          alertLeadMin: existing.net?.alertLeadMin ?? pm.net.alertLeadMin,
        }
      : undefined,
  }
  return patch
}

/** Install (or re-install) a pack into the bank: its channels land (deduped on
 * freq+mode+tone) in a group named after the pack, tagged source 'curated'.
 *
 * Idempotent, and a re-install RECONCILES — a channel already present is refreshed
 * from the pack, so a corrected net time or note in a later Nexus release actually
 * reaches an operator who installed the pack earlier. Only rows the pack still owns
 * (`source: 'curated'`) are touched: editing a row in the Memories UI stamps it
 * `source: 'user'` and a pack never overwrites it again.
 *
 * KNOWN LIMIT: identity is freq+mode+tone, so a pack entry whose FREQUENCY changes
 * reads as a new channel — the corrected row is added and the stale one is left in
 * place for the operator to delete. Fixing that needs a stable per-entry id, which
 * no pack has yet needed (the volatile field is a net's time, which is content-only
 * and reconciles correctly). Revisit if a pack ever moves a frequency.
 *
 * Returns the counts added + updated (both 0 = genuinely already up to date). */
export function importPack(
  bank: MemoriesBank,
  pack: Pack,
): { bank: MemoriesBank; added: number; updated: number } {
  let b = bank
  let group = b.groups.find((g) => g.name === pack.name)
  if (!group) {
    b = addGroup(b, pack.name)
    group = b.groups.find((g) => g.name === pack.name)
  }
  const gid = group?.id
  let added = 0
  let updated = 0
  for (const pm of pack.memories) {
    const probe = coerceMemory({ ...pm, id: 'probe' })
    if (!probe) continue
    const key = memoryKey(probe)
    const existing = b.memories.find((m) => memoryKey(m) === key)
    if (existing) {
      // A channel two packs share (e.g. FT8 14.074 in both Digital and POTA) is not
      // re-added, but it MUST still join THIS pack's group — otherwise the second pack's
      // group would silently be missing the shared channels.
      if (gid && !existing.groups.includes(gid)) {
        b = updateMemory(b, existing.id, { groups: [...existing.groups, gid] })
      }
      if (existing.source !== 'curated') continue // the operator owns this row now
      // Count an update only when the row actually changed, so the toast can't claim
      // work it didn't do. Compare after the group join above, which is not an update.
      const before = JSON.stringify(b.memories.find((m) => m.id === existing.id))
      b = updateMemory(b, existing.id, packContentPatch(pm, existing))
      if (JSON.stringify(b.memories.find((m) => m.id === existing.id)) !== before) updated++
    } else {
      b = addMemory(b, { ...pm, groups: gid ? [gid] : [], source: 'curated' })
      added++
    }
  }
  return { bank: b, added, updated }
}
