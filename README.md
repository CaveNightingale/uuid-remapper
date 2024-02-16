# UUID Remapper
[![Coverage Status](https://coveralls.io/repos/github/CaveNightingale/uuid-remapper/badge.svg?branch=master)](https://coveralls.io/github/CaveNightingale/uuid-remapper?branch=master)

This is a simple tool to remap UUIDs in minecraft worlds. 

It follows heuristics to find UUIDs in the world files and remap them to new ones.

## Build And Install

```sh
cargo install --path .
```

## Usage

See the help message for usage information:
```sh
uuid-remapper /path/to/world csv /path/to/player-old-uuid-new-uuid.csv
uuid-remapper /path/to/world json /path/to/player-old-uuid-new-uuid.json
uuid-remapper /path/to/world list-to-online /path/to/player-list.txt # This will use the Mojang API to get the new UUIDs
uuid-remapper /path/to/world list-to-offline /path/to/player-list.txt # This will use the Mojang API to get the old UUIDs
uuid-remapper /path/to/world usercache-to-online /path/to/usercache.json # Same as list-to-online, but uses the usercache file in the server directory
uuid-remapper /path/to/world usercache-to-offline /path/to/usercache.json # Same as list-to-offline, but uses the usercache file in the server directory
uuid-remapper /path/to/world offline-rename-csv /path/to/player-old-name-new-name.csv
uuid-remapper --help
```

When you are asked to confirm the information (Mainly the function used to remap, and the path of the world), you must answer `yes` (case-insensitive) to proceed. Make sure you have a backup of the world before running the tool.

## Algorithm

The main idea is `find` and `replace`.

```
for file in world:
  if file is *.txt, *.json, *.json5:
    for each uuid: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx, xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx in file and filename:
      uuid = f(uuid)
  else if file is *.dat, *.mca, *.mcc:
    for each uuid: {zzzUUIDMost: xxxL, zzzUUIDLeast: xxxL}, [I; xx, xx, xx, xx] in uncompressed file:
      uuid = f(uuid)
    for each uuid: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx, xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx in filename:
      uuid = f(uuid)
```