#!/usr/bin/env python3
"""刷新定价快照 —— 抓取、匹配、条件档位都在 wb_pricing.py 里。

本脚本只做两件 wb_pricing 自己不适合做的事：

  1. 把快照写成 pricing/pricing.json（本地运行时优先读它）；
  2. --embed：把同一份快照回写进 wb_pricing.py 的内嵌副本，让 Docker 镜像
     （只 COPY wb_*.py，不带 pricing/ 目录）自带一份价格。

网关运行时**不依赖本脚本**：它按面板上配置的间隔自己抓取，并把每一版追加
进 pricing-history.jsonl（见 wb_pricing.PriceRefresher）；每条请求按它发生
时生效的那一版计价。本脚本用于首次内置快照、以及离线生成/更新内置价。

用法：
    python _fetch_pricing.py            # 抓取并写入 pricing/pricing.json
    python _fetch_pricing.py --dry-run  # 只打印，不写文件
    python _fetch_pricing.py --embed    # 同时回写 wb_pricing.py 的内嵌副本
    python _fetch_pricing.py --extra-ids-file ids.txt
                                        # 额外覆盖文件里的模型（每行一个），
                                        # 网关运行时用 live 目录做同样的事

抓取失败时沿用 pricing/pricing.json 里已有的价，而不是写出一份空表。
本脚本默认只覆盖内置静态目录：它不该为了取价去连网关，需要额外 id 时由
调用方用 --extra-ids-file 显式给出（没有网络依赖）。
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import wb_pricing

OUT = os.path.join(HERE, "pricing", "pricing.json")
EMBED_TARGET = os.path.join(HERE, "wb_pricing.py")


def extra_ids_from_file(path):
    """读一份「每行一个模型 id」的清单；空行与 # 注释忽略。"""
    ids = []
    with open(path, encoding="utf-8") as fh:
        for line in fh:
            mid = line.strip()
            if mid and not mid.startswith("#") and mid not in ids:
                ids.append(mid)
    return ids


def load_existing_models():
    """读回上次生成的快照里的 models（若存在），供离线降级使用。"""
    try:
        with open(OUT, encoding="utf-8") as fh:
            return (json.load(fh) or {}).get("models") or {}
    except Exception:
        return {}


def embed(text, count):
    """把快照回写进 wb_pricing.py 的内嵌副本。

    只替换 _JSON = r'''...''' 的内容，文件其余部分原样保留；找不到标记时报错
    退出，而不是把一个手工改过的文件悄悄写坏。
    """
    with open(EMBED_TARGET, encoding="utf-8") as fh:
        src = fh.read()
    start_marker = "_JSON = r'''\n"
    end_marker = "\n'''"
    start = src.find(start_marker)
    if start < 0:
        raise SystemExit("cannot find the inline _JSON block in %s" % EMBED_TARGET)
    start += len(start_marker)
    end = src.find(end_marker, start)
    if end < 0:
        raise SystemExit("cannot find the end of the inline _JSON block in %s"
                         % EMBED_TARGET)
    with open(EMBED_TARGET, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(src[:start] + text + src[end:])
    print("embedded %d models into %s" % (count, EMBED_TARGET))


def main():
    dry = "--dry-run" in sys.argv
    want_embed = "--embed" in sys.argv
    extra_ids = []
    if "--extra-ids-file" in sys.argv:
        try:
            ids_path = sys.argv[sys.argv.index("--extra-ids-file") + 1]
            extra_ids = extra_ids_from_file(ids_path)
        except (IndexError, OSError) as exc:
            raise SystemExit("--extra-ids-file needs a readable path: %s" % exc)
    previous = None
    try:
        or_models = wb_pricing.fetch_openrouter()
    except Exception as exc:
        print("warn: openrouter fetch failed (%s); reusing previous prices" % exc,
              file=sys.stderr)
        or_models = None
        previous = load_existing_models()
        if not previous:
            raise SystemExit("openrouter fetch failed and no previous snapshot")
    doc, unpriced, overridden = wb_pricing.build_snapshot(
        or_models, previous, extra_ids=extra_ids)
    text = json.dumps(doc, ensure_ascii=False, indent=2)
    if dry:
        print(text)
    else:
        os.makedirs(os.path.dirname(OUT), exist_ok=True)
        with open(OUT, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")
        print("wrote %s (%d models)" % (OUT, len(doc["models"])))
        if want_embed:
            embed(text, len(doc["models"]))
    print("matched via OVERRIDES: %d (%s)"
          % (len(overridden), ", ".join(overridden) or "-"))
    if unpriced:
        print("no pricing for %d model(s): %s"
              % (len(unpriced), ", ".join(unpriced)), file=sys.stderr)


if __name__ == "__main__":
    main()
