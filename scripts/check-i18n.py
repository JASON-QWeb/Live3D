# -*- coding: utf-8 -*-
"""Lists console text that has no English in web/i18n-en.json.

Checks the fixed text in web/index.html, the Chinese strings in the console
scripts, and the messages the Rust backend can show in the console. Run it
from the project root after adding or changing Chinese text:

    python3 scripts/check-i18n.py
"""

import io
import json
import re
import sys
from glob import glob
from html.parser import HTMLParser

CJK = re.compile(u"[一-鿿]")
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')

# Chinese that is content, not interface: lines the avatar speaks, prompts for
# the language model, keywords matched in viewer comments, default settings,
# names inside OBS, and terminal output.
CONTENT = set(u"""
{count}个{gift_name}|谢谢{name}送的{spoken_gift_desc}|谢谢老板送的{spoken_gift_desc}|点歌|灵小柔|{}个{}
{name}送的{qty}|热心老板送的{qty}|以及其他{}位送礼物的朋友|问：|问:|答：|答:|即可|都回复|只回复|仅回复
观众「{name}」说：{}|刚才|你刚|你说的|那你|那这|那换|为什么|接着|真的吗|还有吗|怎么说|展开说说
不是这个意思|可是|但是|不过|不用回复|不回复|无需回复|不需要回复|不用回|略过|直播已结束|暂未开播|未开播
神秘人|Vorela · 数字人|Vorela · 舞台输出|macOS 未能打开控制台浏览器|端口 {port} 已被其他程序占用
同一数据目录已有 Vorela 在运行|无法打开默认浏览器|Vorela 正在 127.0.0.1:{port} 运行
－小柔演唱|-小柔演唱|—小柔演唱|_小柔演唱|（小柔演唱）|(小柔演唱)|小柔演唱|97云烟成雨|101红豆|2002年的第一场雪
自然投入地唱歌，神情柔和灵动，随旋律轻轻晃动|唱完歌微笑看向镜头，神态温柔放松
小明|玫瑰|晚上好呀，唱首歌听听|中文|允许播放
""".replace("\n", "|").split("|")) - {""}
# Shown only as parts of another message, which the check lists instead.
PARTS = {u"参考图", u"姿态复位", u"形象切换", u"云端操作", u"执行中", u"已完成", u"执行失败", u"操作"}


def english_keys():
    with io.open("web/i18n-en.json", encoding="utf-8") as source:
        return set(json.load(source))


def template(s):
    """A Rust format string with its placeholders numbered {0}, {1}, ..."""
    counter = iter(range(100))
    s = s.replace("\\n", "\n").replace('\\"', '"')
    return re.sub(r"\{[^{}]*\}", lambda m: "{%d}" % next(counter), s)


def backend():
    found = {}
    for path in sorted(glob("src/**/*.rs", recursive=True)):
        if path.endswith("douyin/recorded.rs"):
            continue
        text = io.open(path, encoding="utf-8").read()
        tests = re.search(r"#\[cfg\(test\)\]\s*mod tests", text)
        if tests:
            text = text[:tests.start()]
        for n, line in enumerate(text.splitlines(), 1):
            if line.strip().startswith("//") or path.endswith("dialogue.rs") and n in (16, 27):
                continue
            for m in LITERAL.finditer(line):
                s = m.group(1)
                if CJK.search(s) and s not in CONTENT and s not in PARTS and u"【" not in s:
                    found.setdefault(template(s), "%s:%d" % (path, n))
    return found


def page():
    found = {}

    class Page(HTMLParser):
        skip = 0

        def handle_starttag(self, tag, attrs):
            self.skip += tag in ("script", "style")
            for name, value in attrs:
                if name in ("placeholder", "title", "aria-label", "alt", "content") and value and CJK.search(value):
                    found.setdefault(value.strip(), "web/index.html")

        def handle_endtag(self, tag):
            self.skip -= tag in ("script", "style")

        def handle_data(self, data):
            text = " ".join(data.split())
            if not self.skip and CJK.search(text):
                found.setdefault(text, "web/index.html")

    Page().feed(io.open("web/index.html", encoding="utf-8").read())
    return found


def scripts():
    found = {}
    for path in ("web/settings.js", "web/studio.js", "web/online-bgm.js", "web/studio-dance.js"):
        for n, line in enumerate(io.open(path, encoding="utf-8").read().splitlines(), 1):
            if line.strip().startswith("//"):
                continue
            for m in LITERAL.finditer(line):
                s = m.group(1)
                if CJK.search(s) and s not in CONTENT:
                    found.setdefault(s, "%s:%d" % (path, n))
    return found


def main():
    have = english_keys()
    gaps = []
    for name, strings in (("page", page()), ("scripts", scripts()), ("backend", backend())):
        missing = {k: v for k, v in strings.items() if k not in have}
        print("%-8s %4d strings, %d without English" % (name, len(strings), len(missing)))
        gaps += ["  %s  %s" % (v, json.dumps(k, ensure_ascii=False)) for k, v in missing.items()]
    if gaps:
        print("\n".join(gaps))
        sys.exit(1)


main()
