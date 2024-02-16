# UUID Remapper

测试徽章：[![Test](https://github.com/CaveNightingale/uuid-remapper/actions/workflows/rust.yml/badge.svg)](https://github.com/CaveNightingale/uuid-remapper/actions/workflows/rust.yml)[![Coverage](https://coveralls.io/repos/github/CaveNightingale/uuid-remapper/badge.svg?branch=master)](https://coveralls.io/github/CaveNightingale/uuid-remapper?branch=master)

这是一个简单的工具，用于重映射 Minecraft 世界中的 UUID。

它使用一种启发式的查找替换方法，来找到世界文件中的 UUID 并将其替换为新的。不保证所有 UUID 都能被找到和替换。

## 构建和安装

```sh
cargo install --path .
```

## 用法

查看帮助信息以获取用法信息：

```sh
uuid-remapper /path/to/world csv /path/to/player-old-uuid-new-uuid.csv
uuid-remapper /path/to/world csv /path/to/player-old-uuid-new-uuid.csv
uuid-remapper /path/to/world json /path/to/player-old-uuid-new-uuid.json
uuid-remapper /path/to/world list-to-online /path/to/player-list.txt # 使用 Mojang API 获取新的 UUID
uuid-remapper /path/to/world list-to-offline /path/to/player-list.txt # 使用 Mojang API 获取旧的 UUID
uuid-remapper /path/to/world usercache-to-online /path/to/usercache.json # 与 list-to-online 相同，但使用服务器目录中的 usercache 文件格式作为输入（而不是一行一个玩家名称）
uuid-remapper /path/to/world usercache-to-offline /path/to/usercache.json # 与 list-to-offline 相同，但使用服务器目录中的 usercache 文件格式作为输入（而不是一行一个玩家名称）
uuid-remapper /path/to/world offline-rename-csv /path/to/player-old-name-new-name.csv
uuid-remapper --help
```

`-t` 选项可以指定线程数。默认为 20，这可能会榨干你的 CPU 导致死机。请根据你的 CPU 核心数来调整这个值。

当你被要求确认（主要是重映射函数和世界路径）时，你必须回答 `yes`（不区分大小写）才能继续。确保在运行工具之前备份世界。`-y` 选项可以自动回答 `yes` 。`-n` 选项可以自动回答 `no`。

## 算法
* 对于文本文件（后缀为txt、json、json5），匹配`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`和`xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx`的 UUID。
* 对于 NBT 文件及其变种（后缀为dat、mca、mcc），匹配 NBT 中`{zzzUUIDMost: xxxL, zzzUUIDLeast: xxxL}`和`[I; xx, xx, xx, xx]`的 UUID，其中`zzz`是任意字符串，上述格式为 SNBT 格式，实际匹配时使用 NBT （也就是二进制）格式。
* 上述两种类型，文件名中的 UUID 也会被匹配，规则与文本文件相同。
* 并不能保证所有 UUID 都能被找到和替换，例如原始 JSON 文本中的 UUID 选择器中的 UUID，以及某些模组使用的 sqlite 文件中的 UUID，都不会被找到和替换。
