#!/usr/bin/env python3
"""判定 Codex Usage Overlay 悬浮窗"看不见"时属于哪一类。

只依赖 Python 标准库（ctypes），不需要安装任何包。

用法：
    python scripts/diagnose-overlay-visibility.py            # 采样一次，打印结论
    python scripts/diagnose-overlay-visibility.py --watch     # 持续监视，状态变化时打一行
    python scripts/diagnose-overlay-visibility.py --watch --seconds 600

判定结果：
    HIDDEN      窗口真的被 hide 了（Tauri 的 hide() / 托盘切换），is_visible=False
    CLOAKED     窗口还在，但被 DWM 隐藏（显示桌面 Win+D、虚拟桌面切换、锁屏）
    OFFSCREEN   窗口坐标不在任何显示器工作区内（多屏/远程桌面改分辨率后漂走）
    ZERO-SIZE   窗口尺寸被压到 0
    COVERED     窗口 visible 且置顶，但上面压着别的窗口（置顶失效/被更强的置顶抢走）
    SHOWN       正常显示
"""

import argparse
import ctypes
import ctypes.wintypes as w
import datetime
import sys

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi", use_last_error=True)

PROCESS_NAME = "codex-usage-overlay.exe"

GW_HWNDPREV = 3
GWL_STYLE = -16
GWL_EXSTYLE = -20
WS_VISIBLE = 0x10000000
WS_EX_TOPMOST = 0x00000008
WS_EX_TRANSPARENT = 0x00000020
WS_EX_LAYERED = 0x00080000
DWMWA_CLOAKED = 14


class MONITORINFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", w.DWORD),
        ("rcMonitor", w.RECT),
        ("rcWork", w.RECT),
        ("dwFlags", w.DWORD),
    ]


def _rect(hwnd):
    r = w.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def _title(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


def _class(hwnd):
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def _pid(hwnd):
    pid = w.DWORD(0)
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def _process_name(pid):
    handle = kernel32.OpenProcess(0x1000, False, pid)
    if not handle:
        return ""
    buf = ctypes.create_unicode_buffer(1024)
    size = w.DWORD(1024)
    ok = kernel32.QueryFullProcessImageNameW(handle, 0, buf, ctypes.byref(size))
    kernel32.CloseHandle(handle)
    return buf.value.split("\\")[-1] if ok else ""


def _enum_windows():
    windows = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, w.HWND, w.LPARAM)
    def _callback(hwnd, _lparam):
        windows.append(hwnd)
        return True

    user32.EnumWindows(_callback, 0)
    return windows


def _monitors():
    monitors = []

    @ctypes.WINFUNCTYPE(ctypes.c_int, w.HMONITOR, w.HDC, ctypes.POINTER(w.RECT), w.LPARAM)
    def _callback(hmonitor, _hdc, _lprc, _lparam):
        info = MONITORINFO()
        info.cbSize = ctypes.sizeof(MONITORINFO)
        user32.GetMonitorInfoW(hmonitor, ctypes.byref(info))
        monitors.append(
            {
                "monitor": (info.rcMonitor.left, info.rcMonitor.top, info.rcMonitor.right, info.rcMonitor.bottom),
                "work": (info.rcWork.left, info.rcWork.top, info.rcWork.right, info.rcWork.bottom),
                "primary": bool(info.dwFlags & 1),
            }
        )
        return 1

    user32.EnumDisplayMonitors(0, 0, _callback, 0)
    return monitors


def _intersects(a, b):
    return a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3]


def _inside(rect, area):
    return rect[0] >= area[0] and rect[1] >= area[1] and rect[2] <= area[2] and rect[3] <= area[3]


def _cloaked(hwnd):
    value = w.DWORD(0)
    hr = dwmapi.DwmGetWindowAttribute(w.HWND(hwnd), DWMWA_CLOAKED, ctypes.byref(value), ctypes.sizeof(value))
    return value.value if hr == 0 else 0


def find_overlay():
    """返回悬浮窗 hwnd：优先标题，其次同进程里最小的 Tauri Window。"""
    candidates = []
    for hwnd in _enum_windows():
        if _process_name(_pid(hwnd)).lower() != PROCESS_NAME:
            continue
        if _class(hwnd) != "Tauri Window":
            continue
        candidates.append(hwnd)
    for hwnd in candidates:
        if "Usage Overlay" in _title(hwnd):
            return hwnd
    fallback = [h for h in candidates if (_rect(h)[2] - _rect(h)[0]) < 1000]
    return fallback[0] if fallback else (candidates[0] if candidates else None)


def probe():
    hwnd = find_overlay()
    if hwnd is None:
        return {
            "verdict": "NO-WINDOW",
            "detail": f"没有找到 {PROCESS_NAME} 的悬浮窗：进程没在跑，或窗口已被销毁",
        }

    rect = _rect(hwnd)
    style = user32.GetWindowLongW(hwnd, GWL_STYLE)
    exstyle = user32.GetWindowLongW(hwnd, GWL_EXSTYLE)
    visible = bool(user32.IsWindowVisible(hwnd))
    minimized = bool(user32.IsIconic(hwnd))
    cloaked = _cloaked(hwnd)
    topmost = bool(exstyle & WS_EX_TOPMOST)
    clickthrough = bool(exstyle & WS_EX_TRANSPARENT)

    monitors = _monitors()
    width = rect[2] - rect[0]
    height = rect[3] - rect[1]
    size = (width, height)
    on_monitor = [m for m in monitors if _intersects(rect, m["monitor"])]
    inside_work = any(_inside(rect, m["work"]) for m in monitors)

    cover = []
    handle = user32.GetWindow(hwnd, GW_HWNDPREV)
    guard = 0
    while handle and guard < 200:
        if user32.IsWindowVisible(handle):
            other = _rect(handle)
            if other[2] > other[0] and other[3] > other[1] and _intersects(other, rect):
                other_ex = user32.GetWindowLongW(handle, GWL_EXSTYLE)
                cover.append(
                    {
                        "title": _title(handle) or _class(handle),
                        "rect": other,
                        "topmost": bool(other_ex & WS_EX_TOPMOST),
                    }
                )
        handle = user32.GetWindow(handle, GW_HWNDPREV)
        guard += 1

    if not visible:
        verdict = "HIDDEN"
        detail = "窗口被 hide 了（Tauri hide() / 托盘切换 / 启动时持久化的 hidden=true）"
    elif cloaked:
        verdict = "CLOAKED"
        detail = f"窗口被 DWM 隐藏（cloaked={cloaked}）：显示桌面、虚拟桌面切换或锁屏"
    elif minimized:
        verdict = "MINIMIZED"
        detail = "窗口被最小化"
    elif width <= 0 or height <= 0:
        verdict = "ZERO-SIZE"
        detail = f"窗口尺寸为 {size}"
    elif not on_monitor:
        verdict = "OFFSCREEN"
        detail = "窗口坐标不落在任何显示器上（换了分辨率/显示器拓扑后漂到屏幕外）"
    elif not inside_work:
        verdict = "OFFSCREEN"
        detail = "窗口部分或全部落在工作区之外（贴边/被任务栏或屏幕边缘裁掉）"
    elif cover:
        verdict = "COVERED"
        detail = "窗口可见且置顶，但 Z 序上方有窗口压着它"
    else:
        verdict = "SHOWN"
        detail = "正常显示"

    return {
        "verdict": verdict,
        "detail": detail,
        "hwnd": hwnd,
        "rect": rect,
        "size": size,
        "visible": visible,
        "topmost": topmost,
        "clickthrough": clickthrough,
        "cloaked": cloaked,
        "on_monitor": bool(on_monitor),
        "inside_work": inside_work,
        "cover": cover,
        "monitors": monitors,
    }


def report(result):
    stamp = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    print(f"[{stamp}] {result['verdict']}: {result['detail']}")
    if result["verdict"] == "NO-WINDOW":
        return
    print(f"  hwnd={result['hwnd']} rect={result['rect']} size={result['size']}")
    print(
        f"  visible={result['visible']} topmost={result['topmost']} "
        f"clickthrough={result['clickthrough']} cloaked={result['cloaked']} "
        f"on_monitor={result['on_monitor']} inside_work={result['inside_work']}"
    )
    for monitor in result["monitors"]:
        print(
            f"  monitor rc={monitor['monitor']} work={monitor['work']} primary={monitor['primary']}"
        )
    for item in result["cover"]:
        print(f"  above: '{item['title'][:40]}' rect={item['rect']} topmost={item['topmost']}")


def main():
    parser = argparse.ArgumentParser(description="诊断 Codex Usage Overlay 悬浮窗是否真的消失了")
    parser.add_argument("--watch", action="store_true", help="持续监视，只在状态变化时输出")
    parser.add_argument("--seconds", type=int, default=0, help="监视时长，0 表示一直跑到 Ctrl+C")
    parser.add_argument("--interval", type=float, default=0.5, help="采样间隔（秒）")
    args = parser.parse_args()

    try:
        sys.stdout.reconfigure(encoding="utf-8")
    except Exception:
        pass

    if not args.watch:
        report(probe())
        return

    import time

    deadline = time.time() + args.seconds if args.seconds > 0 else None
    last = None
    print("监视中，状态变化时输出一行；Ctrl+C 结束。", flush=True)
    while True:
        result = probe()
        key = (
            result["verdict"],
            result.get("rect"),
            result.get("visible"),
            result.get("topmost"),
            tuple(item["title"] for item in result.get("cover", [])),
        )
        if key != last:
            report(result)
            sys.stdout.flush()
            last = key
        if deadline and time.time() >= deadline:
            break
        time.sleep(args.interval)


if __name__ == "__main__":
    main()
