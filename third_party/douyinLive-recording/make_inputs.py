# -*- coding: utf-8 -*-
"""Realistic inputs for recording douyinLive with rust-anything.

Writes webcastsign.jsonl, sign.jsonl and golden.jsonl in the current folder,
one {"fn": name, "args": [...]} per line. The page fixtures come from
douyinLive's own tests (douyin_test.go) plus a few variations.
"""

import hashlib
import io
import json
import random

random.seed(7)
UA = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
    "curl/8.0", "", "Mozilla/", "Chrome/", "x Chrome/136.0.7103.59 y",
]
ROOMS = [("7659792511015177001", "7659797852999091746"), ("7312345678901234567", "101220697463"),
         ("", ""), ("1", "2"), (u"房间", u"用户 id")]


def joined(r, u):
    fields = [("live_id", "1"), ("aid", "6383"), ("version_code", "180800"), ("webcast_sdk_version", "1.0.15"),
              ("room_id", r), ("sub_room_id", ""), ("sub_channel_id", ""), ("did_rule", "3"),
              ("user_unique_id", u), ("device_platform", "web"), ("device_type", ""), ("ac", ""),
              ("identity", "audience")]
    return ",".join(u"%s=%s" % kv for kv in fields)


def write(name, rows):
    with io.open(name, "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + u"\n")


stubs = [hashlib.md5(joined(r, u).encode("utf-8")).hexdigest() for r, u in ROOMS]
stubs += ["0" * 32, "f" * 32, "ABCDEF0123456789abcdef0123456789", "0" * 31, "g" * 32, "0" * 34]
rows = []
for s in stubs:
    for c in [0, 1, 63, 64, 127, 200, 255]:
        for flag in [False, True]:
            rows.append({"fn": "SignWithValues", "args": [s, c, flag, random.randrange(255), random.randrange(255)]})
    rows.append({"fn": "SignWithValues", "args": [s, 5, True, 0, 254]})
    rows.append({"fn": "SignWithValues", "args": [s, 5, False, 254, 0]})
write("webcastsign.jsonl", rows)

rows = [{"fn": "SM3Hex", "args": ["a" * n]} for n in list(range(140)) + [255, 256, 511, 512, 1000, 4096]]
rows += [{"fn": "SM3Hex", "args": [s]} for s in ["abc", "abcd" * 16, u"日本語", u"😀" * 20, "aid=6383&app_name=douyin_web"]]
params = [
    "aid=6383&app_name=douyin_web&live_id=1&device_platform=web&language=zh-CN&enter_from=link_share&cookie_enabled=true"
    "&screen_width=1920&screen_height=1080&browser_language=zh-CN&browser_platform=Win32&browser_name=Chrome"
    "&browser_version=133.0.0.0&os_name=Windows&os_version=10&web_rid=516466932480&room_id_str=&enter_source="
    "&is_need_double_stream=false&insert_task_id=&live_reason=&msToken=abc-_DEF",
    "", "a=1", "x" * 500, u"q=日本語&r=😀",
]
for p in params:
    for ua in UA[:4]:
        for t in [0, 1, 255, 1719159695790, 1763800000123, 4294967295, 4294967296, 1763800000123 + 2 ** 32]:
            rows.append({"fn": "AbSignAt", "args": [p, ua, t]})
write("sign.jsonl", rows)

rows = [{"fn": "GoldenWSStub", "args": [r, u]} for r, u in ROOMS]
cursors = ["d-1_u-1_fh-7383731312643626035_t-1719159695790_r-1", "", "r-1&x=y"]
exts = ["internal_src:dim|wss_push_room_id:1|wss_push_did:2|first_req_ms:3|fetch_time:3|seq:1|wss_info:0-3-0-0|wrds_v:7382620942951772256", "", "a b&c"]
sigs = ["DFSzswVOmh4ZK0ab", "a b+c/d", ""]
for r, u in ROOMS[:3]:
    for ua in UA:
        rows.append({"fn": "GoldenWSQuery", "args": [r, u, ua, random.choice(cursors), random.choice(exts), random.choice(sigs), 1920, 1080]})
        rows.append({"fn": "GoldenIMFetchQuery", "args": [r, u, ua, "tok+en/=&x y", 1536, 864]})
        for known in [False, True]:
            rows.append({"fn": "GoldenRoomEnterQuery", "args": ["516466932480", r, ua, "tok+en/=&x y", known, 2560, 1440]})
rows.append({"fn": "GoldenRoomEnterQuery", "args": ["a b&c", "9", UA[0], "", True, 1366, 768]})
for r, u, ms in [("7659792511015177001", "7659797852999091746", 1763800000123), ("", "", 0), ("1", "2", -5)]:
    rows.append({"fn": "GoldenInternalExt", "args": [r, u, ms]})
for c in ["", "  ", "webcast5-ws-web-lf.douyin.com", "wss://webcast5-ws-web-hl.douyin.com/webcast/im/push/v2/",
          "https://a.example.com/", "http://a.example.com", "ws://h/webcast/im/push/v2", "a.com,b.com", "a.com;b.com",
          " wss://x.douyin.com// ", "wss://x/webcast/im/push/v2/extra"]:
    rows.append({"fn": "GoldenPushURL", "args": [c]})
for h in ["", "ping-interval=15;", "ping-interval=15", " PING-INTERVAL = 20 ; x=1", "x=1;ping-interval=0",
          "ping-interval=-3", "ping-interval=abc", "a;b;ping-interval=30;"]:
    rows.append({"fn": "GoldenPingInterval", "args": [h]})
rows += [{"fn": "GoldenChromeVersion", "args": [ua]} for ua in UA]
rows += [{"fn": "GoldenQueryEscape", "args": [v]} for v in ["", "abc", "a b", "a+b", "~-._", u"日本", "a&b=c/d?e#f", u"😀", "%41", "'\"!*()"]]
rows += [{"fn": "GoldenValidateLiveID", "args": [v]} for v in ["516466932480", " 516466932480 ", "", "a_b-C9", "a.b", "a/b", "x" * 128, "x" * 129, u"日本", "../ws"]]

enter = [
    '{"status_code":0,"data":{"data":[{"id_str":"7659792511015177001","status":2,"title":"t1","owner":{"id_str":"101","nickname":"n1","avatar_thumb":{"url_list":["a0","a1","a2"]}}}],"user":{"id_str":"202","nickname":"u1","avatar_thumb":{"url_list":["b0"]}}}}',
    '{"data":{"room":{"id_str":"room-id","status":4,"title":"room-title","owner":{"id_str":"owner-id","nickname":"owner-name","avatar_thumb":{"url_list":["avatar-0","avatar-1","avatar-2"]}}}}}',
    '{"data":{"data":[{"id":7659792511015177001,"status":"2","owner_user_id_str":"55"}]}}',
    '{"data":{"enter_room_id":"77","user":{"id":12345}}}',
    '{"data":{"data":[{"id_str":"r"}]}}',
    '{"data":{}}', '', 'not json', '{"data":{"data":[{"id_str":"r","owner":{"id":0}}]}}',
    u'{"data":{"data":[{"id_str":"7659","status":2}],"user":{"id_str":"","id":"88","nickname":"日本😀"}}}',
]
rows += [{"fn": "GoldenRoomEnter", "args": [b]} for b in enter]
pages = [
    'anchor_id_str=64995611209&amp;enter_method=direct_open&amp;room_id=7659792511015177001&amp;sec_anchor_id=abc',
    '{"room_id":"7659792511015177001","status":2}',
    '\\"roomStore\\":{\\"roomInfo\\":{\\"room\\":{\\"id_str\\":\\"7659792511015177001\\",\\"status\\":2}}}',
    u'self.__pace_f.push([1,"{\\"id_str\\":\\"7659631024187493163\\",\\"status\\":2,\\"status_str\\":\\"2\\",\\"title\\":\\"直播标题\\",\\"user_count_str\\":\\"68\\","])',
    u'<div id="gift_effect_bg_7659772543859215154"></div><span>直播已结束</span>',
    u'self.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{\\"room\\":{\\"id_str\\":\\"7659786040097426226\\",\\"status\\":2,\\"status_str\\":\\"2\\",\\"title\\":\\"梦三国娱乐解说\\",\\"owner\\":{\\"id_str\\":\\"101220697463\\",\\"nickname\\":\\"走秀（梦三国解说）\\",\\"avatar_thumb\\":{\\"url_list\\":[\\"https://p3.douyinpic.com/avatar.jpeg\\",\\"https://p11.douyinpic.com/avatar.jpeg\\"]}}}}}}"])',
    'self.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{},\\"liveStatus\\":\\"normal\\"}}"])\nself.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{\\"roomId\\":\\"\\",\\"web_rid\\":\\"32536162943\\",\\"anchor\\":{\\"id_str\\":\\"3872957772872119\\",\\"nickname\\":\\"anchor-only-name\\",\\"avatar_thumb\\":{\\"url_list\\":[\\"https://p3.douyinpic.com/a.jpeg\\"]}}}}}"])',
    'self.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{\\"roomId\\":\\"7659700000000000000\\",\\"web_rid\\":\\"32536162943\\",\\"anchor\\":{\\"id_str\\":\\"3872957772872119\\",\\"nickname\\":\\"?????\\",\\"avatar_thumb\\":{\\"url_list\\":[\\"https://p3.douyinpic.com/a.jpeg\\"]}}}}}"])',
    'self.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{\\"web_rid\\":\\"386395296025\\",\\"anchor\\":{\\"id_str\\":\\"68252455312\\",\\"nickname\\":\\"CACA??\\"}}}}"])\nself.__pace_f.push([1,"{\\"roomStore\\":{\\"roomInfo\\":{\\"room\\":{\\"id_str\\":\\"7659772543859215154\\",\\"status\\":4,\\"status_str\\":\\"4\\",\\"title\\":\\"?????\\"},\\"anchor\\":{\\"id_str\\":\\"68252455312\\",\\"nickname\\":\\"CACA??\\"}}}}}"])',
    'setPageViewLog({"odin":"{\\"user_id\\":\\"1561766825835499\\",\\"user_unique_id\\":\\"7659797852999091746\\"}"});',
    '<html><script>window.__log={"user_unique_id":"7659776308930922010"}; window.endpoint="/webcast/room/web/enter/";</script><body></body></html>',
    u'self.__pace_f.push([1,"{\\"id_str\\":\\"7659772543859215154\\",\\"status\\":4,\\"status_str\\":\\"4\\",\\"title\\":\\"CACA呆夫\\",\\"user_count_str\\":\\"\\","])',
    u'<title>CACA呆夫（无畏契约）的抖音直播间 - 抖音直播</title><div id="gift_effect_bg_7659772543859215154"></div><span>直播已结束</span>',
    '', '<html></html>', u'暂未开播',
    u'{"roomStore":{"roomInfo":{"room":{"id_str":"7659000000000000001","status":2,"owner":{"id":101,"nickname":"含\\"引号\\"的名字"}},"anchor":{}}}} user_unique_id=7659797852999091746',
    '{"roomStore":{"roomInfo":{"anchor":{}}}}',
    '{"room":{"id_str":"7650000000000000002","status":4,"title":"x"}} room_id=7650000000000000002',
    '\\\\"roomStore\\\\":{\\\\"roomInfo\\\\":{\\\\"room\\\\":{\\\\"id_str\\\\":\\\\"7659792511015177009\\\\",\\\\"status\\\\":2}}} \\\\u0026x',
    'room_id%3D7659792511015177003&user_unique_id%3D7659797852999091749',
    '"roomId": 7659792511015177004 "status":2',
]
rows += [{"fn": "GoldenLivePage", "args": [p]} for p in pages]
write("golden.jsonl", rows)
print("wrote webcastsign.jsonl, sign.jsonl, golden.jsonl")
