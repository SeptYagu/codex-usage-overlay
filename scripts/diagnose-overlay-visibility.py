#!/usr/bin/env python3
r"""判定 Codex Usage Overlay 悬浮窗"看不见"时属于哪一类。

只依赖 Python 标准库（ctypes），只支持 Windows。

用法：
    python scripts/diagnose-overlay-visibility.py              # 采样一次，打印结论和明细
    python scripts/diagnose-overlay-visibility.py --watch      # 持续监视，状态变化时输出一行
    python scripts/diagnose-overlay-visibility.py --watch --seconds 600 --log overlay-visibility.log

判定结果（按优先级从高到低）：
    NO-PROCESS    codex-usage-overlay.exe 没在运行
    NO-WINDOW     进程还在，但悬浮窗已被销毁（v1.9.4 及以前主窗口收到 WM_CLOSE 就会这样：Alt+F4、不带 /F 的 taskkill 等）
    HIDDEN        窗口被 hide（托盘/菜单切换，或启动时沿用 dock-state.json 的 hidden=true）
    CLOAKED       窗口被 DWM cloak（例如属于另一个虚拟桌面），IsWindowVisible 仍为 true
    MINIMIZED     窗口被最小化
    ZERO-SIZE     窗口宽或高为 0
    OFFSCREEN     窗口完全不在任何显示器上，或有一部分落在工作区之外
    WEBVIEW-GONE  窗口还在，但 WebView2 内容窗口没了（msedgewebview2.exe 已退出），透明窗口什么也不画
    COVERED       Z 序上方有可见窗口压着它
    NOT-TOPMOST   没被压住，但已经失去置顶
    SHOWN         正常显示

坐标一律是物理像素（脚本以 Per-Monitor DPI Aware V2 运行），可以直接和
%LOCALAPPDATA%\CodexUsageOverlay\window-position.json 里保存的锚点对比。
"""

import argparse
import ctypes
import datetime
import sys
import time

if sys.platform != "win32":
    sys.exit("此脚本只支持 Windows。")

import ctypes.wintypes as w  # noqa: E402  只在 Windows 上导入

PROCESS_NAME = "codex-usage-overlay.exe"
WEBVIEW_PROCESS_NAME = "msedgewebview2.exe"
# src-tauri/tauri.conf.json 中 label="main" 的窗口标题；运行期不会改。
OVERLAY_TITLE = "Codex Usage Overlay"
WINDOW_CLASS = "Tauri Window"

GW_HWNDPREV = 3
GWL_EXSTYLE = -20
WS_EX_TOPMOST = 0x00000008
WS_EX_TRANSPARENT = 0x00000020
WS_EX_LAYERED = 0x00080000
LWA_ALPHA = 0x00000002
DWMWA_CLOAKED = 14
MONITORINFOF_PRIMARY = 0x00000001
TH32CS_SNAPPROCESS = 0x00000002
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2 = -4
PROCESS_PER_MONITOR_DPI_AWARE = 2

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi")


class MONITORINFO(ctypes.Structure):
    _fields_ = [
        ("cbSize", w.DWORD),
        ("rcMonitor", w.RECT),
        ("rcWork", w.RECT),
        ("dwFlags", w.DWORD),
    ]


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", w.DWORD),
        ("cntUsage", w.DWORD),
        ("th32ProcessID", w.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", w.DWORD),
        ("cntThreads", w.DWORD),
        ("th32ParentProcessID", w.DWORD),
        ("pcPriClassBase", w.LONG),
        ("dwFlags", w.DWORD),
        ("szExeFile", w.WCHAR * 260),
    ]


WNDENUMPROC = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
MONITORENUMPROC = ctypes.WINFUNCTYPE(w.BOOL, w.HMONITOR, w.HDC, ctypes.POINTER(w.RECT), w.LPARAM)


def _declare(function, restype, *argtypes):
    # 不声明签名时 ctypes 按 32 位 int 传递和返回句柄，64 位 Python 上会截断 HWND/HANDLE。
    function.restype = restype
    function.argtypes = argtypes


_declare(user32.EnumWindows, w.BOOL, WNDENUMPROC, w.LPARAM)
_declare(user32.EnumChildWindows, w.BOOL, w.HWND, WNDENUMPROC, w.LPARAM)
_declare(user32.EnumDisplayMonitors, w.BOOL, w.HDC, ctypes.POINTER(w.RECT), MONITORENUMPROC, w.LPARAM)
_declare(user32.GetMonitorInfoW, w.BOOL, w.HMONITOR, ctypes.POINTER(MONITORINFO))
_declare(user32.GetWindow, w.HWND, w.HWND, w.UINT)
_declare(user32.GetWindowRect, w.BOOL, w.HWND, ctypes.POINTER(w.RECT))
_declare(user32.GetWindowTextLengthW, ctypes.c_int, w.HWND)
_declare(user32.GetWindowTextW, ctypes.c_int, w.HWND, w.LPWSTR, ctypes.c_int)
_declare(user32.GetClassNameW, ctypes.c_int, w.HWND, w.LPWSTR, ctypes.c_int)
_declare(user32.GetWindowThreadProcessId, w.DWORD, w.HWND, ctypes.POINTER(w.DWORD))
_declare(user32.GetWindowLongW, w.LONG, w.HWND, ctypes.c_int)
_declare(
    user32.GetLayeredWindowAttributes,
    w.BOOL,
    w.HWND,
    ctypes.POINTER(w.COLORREF),
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.POINTER(w.DWORD),
)
_declare(user32.IsWindow, w.BOOL, w.HWND)
_declare(user32.IsWindowVisible, w.BOOL, w.HWND)
_declare(user32.IsIconic, w.BOOL, w.HWND)
_declare(kernel32.CreateToolhelp32Snapshot, w.HANDLE, w.DWORD, w.DWORD)
_declare(kernel32.Process32FirstW, w.BOOL, w.HANDLE, ctypes.POINTER(PROCESSENTRY32W))
_declare(kernel32.Process32NextW, w.BOOL, w.HANDLE, ctypes.POINTER(PROCESSENTRY32W))
_declare(kernel32.CloseHandle, w.BOOL, w.HANDLE)
_declare(dwmapi.DwmGetWindowAttribute, ctypes.c_long, w.HWND, w.DWORD, ctypes.c_void_p, w.DWORD)


def enable_dpi_awareness():
    """让窗口和显示器坐标都按物理像素返回，必须在读取任何坐标之前调用。

    Python 进程默认不声明 DPI 感知，Windows 会把其它进程的窗口坐标和显示器矩形按缩放
    比例虚拟化：125% 的 2560x1440 主屏会读成 2048x1152，悬浮窗的 x=771 会读成 617，
    和应用自己保存的物理坐标对不上，相邻显示器之间还会凭空出现"缝隙"。
    """
    try:
        set_context = user32.SetProcessDpiAwarenessContext
    except AttributeError:
        set_context = None
    if set_context is not None:
        set_context.restype = w.BOOL
        set_context.argtypes = [ctypes.c_void_p]
        if set_context(ctypes.c_void_p(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)):
            return True
    try:
        return ctypes.WinDLL("shcore").SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE) == 0
    except (AttributeError, OSError):
        return bool(user32.SetProcessDPIAware())


def _rect(hwnd):
    r = w.RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(r)):
        return (0, 0, 0, 0)
    return (r.left, r.top, r.right, r.bottom)


def _title(hwnd):
    length = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, buf, length + 1)
    return buf.value


def _class(hwnd):
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def _pid(hwnd):
    pid = w.DWORD(0)
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def _exstyle(hwnd):
    return user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & 0xFFFFFFFF


def _cloaked(hwnd):
    value = w.DWORD(0)
    hr = dwmapi.DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, ctypes.byref(value), ctypes.sizeof(value))
    return value.value if hr == 0 else 0


def _fully_transparent(hwnd, exstyle):
    """整体 alpha 为 0 的分层窗口什么也不画，不算遮挡。"""
    if not exstyle & WS_EX_LAYERED:
        return False
    alpha = ctypes.c_ubyte(255)
    flags = w.DWORD(0)
    if not user32.GetLayeredWindowAttributes(hwnd, None, ctypes.byref(alpha), ctypes.byref(flags)):
        # 用 UpdateLayeredWindow 绘制的窗口读不到 alpha，按可能遮挡处理。
        return False
    return bool(flags.value & LWA_ALPHA) and alpha.value == 0


def _process_names():
    """一次进程快照拿到全部 pid -> 可执行文件名（小写），不必逐个窗口 OpenProcess。"""
    names = {}
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if not snapshot or snapshot == INVALID_HANDLE_VALUE:
        return names
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(PROCESSENTRY32W)
        ok = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            names[entry.th32ProcessID] = entry.szExeFile.lower()
            ok = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return names


def _enum_windows():
    windows = []

    @WNDENUMPROC
    def _callback(hwnd, _lparam):
        windows.append(hwnd)
        return True

    user32.EnumWindows(_callback, 0)
    return windows


def _enum_children(hwnd):
    children = []

    @WNDENUMPROC
    def _callback(child, _lparam):
        children.append(child)
        return True

    user32.EnumChildWindows(hwnd, _callback, 0)
    return children


def _monitors():
    monitors = []

    @MONITORENUMPROC
    def _callback(hmonitor, _hdc, _lprc, _lparam):
        info = MONITORINFO()
        info.cbSize = ctypes.sizeof(MONITORINFO)
        if user32.GetMonitorInfoW(hmonitor, ctypes.byref(info)):
            monitors.append(
                {
                    "monitor": (info.rcMonitor.left, info.rcMonitor.top, info.rcMonitor.right, info.rcMonitor.bottom),
                    "work": (info.rcWork.left, info.rcWork.top, info.rcWork.right, info.rcWork.bottom),
                    "primary": bool(info.dwFlags & MONITORINFOF_PRIMARY),
                }
            )
        return True

    user32.EnumDisplayMonitors(None, None, _callback, 0)
    return monitors


def _intersection_area(a, b):
    width = min(a[2], b[2]) - max(a[0], b[0])
    height = min(a[3], b[3]) - max(a[1], b[1])
    return width * height if width > 0 and height > 0 else 0


def _is_overlay(hwnd, app_pids):
    return (
        bool(user32.IsWindow(hwnd))
        and _pid(hwnd) in app_pids
        and _class(hwnd) == WINDOW_CLASS
        and _title(hwnd) == OVERLAY_TITLE
    )


def find_overlay(names, cached=None):
    """返回 (悬浮窗 hwnd 或 None, 本应用的 pid 集合, 本应用其它 Tauri 窗口的标题)。

    只认标题为 OVERLAY_TITLE 的 Tauri 窗口：设置窗口和托盘弹窗（"Tray Menu"）属于同一个
    进程、同一个窗口类，靠尺寸猜会认错。
    """
    app_pids = {pid for pid, name in names.items() if name == PROCESS_NAME}
    if not app_pids:
        return None, app_pids, []
    if cached and _is_overlay(cached, app_pids):
        return cached, app_pids, []
    others = []
    for hwnd in _enum_windows():
        if _pid(hwnd) not in app_pids or _class(hwnd) != WINDOW_CLASS:
            continue
        title = _title(hwnd)
        if title == OVERLAY_TITLE:
            return hwnd, app_pids, []
        others.append(title)
    return None, app_pids, others


def _webview_window_count(hwnd, names):
    """悬浮窗下属于 msedgewebview2.exe 的子窗口数。

    WebView2 的内容窗口（Chrome_WidgetWin_1 / Chrome_RenderWidgetHostHWND 等）由它的
    浏览器进程创建；该进程退出后这些子窗口随之销毁，只剩应用进程自己的宿主窗口。
    """
    return sum(1 for child in _enum_children(hwnd) if names.get(_pid(child)) == WEBVIEW_PROCESS_NAME)


def _windows_above(hwnd, rect, names):
    """Z 序在悬浮窗上方、与它相交、而且真的会画出来的窗口。"""
    above = []
    handle = user32.GetWindow(hwnd, GW_HWNDPREV)
    for _ in range(1000):
        if not handle:
            break
        if user32.IsWindowVisible(handle) and not _cloaked(handle):
            other = _rect(handle)
            exstyle = _exstyle(handle)
            if _intersection_area(other, rect) > 0 and not _fully_transparent(handle, exstyle):
                above.append(
                    {
                        "hwnd": handle,
                        "title": _title(handle) or _class(handle),
                        "process": names.get(_pid(handle), "?"),
                        "rect": other,
                        "topmost": bool(exstyle & WS_EX_TOPMOST),
                        "clickthrough": bool(exstyle & WS_EX_TRANSPARENT),
                    }
                )
        handle = user32.GetWindow(handle, GW_HWNDPREV)
    return above


def probe(cached_hwnd=None):
    names = _process_names()
    monitors = _monitors()
    hwnd, app_pids, others = find_overlay(names, cached_hwnd)
    if not app_pids:
        return {
            "verdict": "NO-PROCESS",
            "detail": f"{PROCESS_NAME} 没在运行（已退出、崩溃、被结束，或正在自动更新重启）",
            "monitors": monitors,
        }
    if hwnd is None:
        seen = "、".join(repr(title) for title in others) or "无"
        return {
            "verdict": "NO-WINDOW",
            "detail": (
                f"进程 {sorted(app_pids)} 还在，但悬浮窗已被销毁（v1.9.4 及以前收到 WM_CLOSE 就会这样，例如"
                f" Alt+F4 或不带 /F 的 taskkill）；托盘切换不会再有反应，只能重启。该进程现有的 Tauri 窗口：{seen}"
            ),
            "pids": sorted(app_pids),
            "monitors": monitors,
        }

    rect = _rect(hwnd)
    width = rect[2] - rect[0]
    height = rect[3] - rect[1]
    area = max(width, 0) * max(height, 0)
    exstyle = _exstyle(hwnd)
    visible = bool(user32.IsWindowVisible(hwnd))
    minimized = bool(user32.IsIconic(hwnd))
    cloaked = _cloaked(hwnd)
    topmost = bool(exstyle & WS_EX_TOPMOST)
    clickthrough = bool(exstyle & WS_EX_TRANSPARENT)
    webview = _webview_window_count(hwnd, names)
    on_screen = sum(_intersection_area(rect, m["monitor"]) for m in monitors)
    # 显示器互不重叠，所以对各工作区的相交面积求和就是落在工作区内的面积；
    # 跨在两块相邻屏幕上的窗口不会被误判为越界。
    in_work = sum(_intersection_area(rect, m["work"]) for m in monitors)
    in_work_percent = in_work * 100 // area if area else 0
    above = _windows_above(hwnd, rect, names) if visible and not minimized and area else []

    if not visible:
        verdict, detail = "HIDDEN", "窗口被 hide 了：托盘/菜单切换，或启动时沿用了 dock-state.json 的 hidden=true"
    elif cloaked:
        verdict, detail = "CLOAKED", f"窗口被 DWM cloak（cloaked={cloaked}），例如属于另一个虚拟桌面"
    elif minimized:
        verdict, detail = "MINIMIZED", "窗口被最小化"
    elif area == 0:
        verdict, detail = "ZERO-SIZE", f"窗口尺寸为 {(width, height)}"
    elif on_screen == 0:
        verdict, detail = "OFFSCREEN", "窗口完全不在任何显示器上（显示器拓扑或分辨率变化后漂走）"
    elif webview == 0:
        verdict, detail = (
            "WEBVIEW-GONE",
            "窗口还在，但没有属于 msedgewebview2.exe 的内容子窗口：WebView2 进程已退出，透明窗口什么也不画。"
            "v1.9.4 及以前只能重启；之后的版本会自动重启，持续出现说明恢复没有生效",
        )
    elif in_work < area:
        verdict, detail = "OFFSCREEN", f"只有 {in_work_percent}% 落在工作区内，其余被屏幕边缘或任务栏裁掉"
    elif above:
        lost = "，而且已经失去置顶" if not topmost else ""
        verdict, detail = "COVERED", f"Z 序上方有 {len(above)} 个可见窗口压着它{lost}"
    elif not topmost:
        verdict, detail = "NOT-TOPMOST", "暂时没被压住，但已失去置顶（WS_EX_TOPMOST 被清除），普通窗口也能盖住它"
    else:
        verdict, detail = "SHOWN", "正常显示"

    return {
        "verdict": verdict,
        "detail": detail,
        "pid": _pid(hwnd),
        "hwnd": hwnd,
        "rect": rect,
        "size": (width, height),
        "visible": visible,
        "minimized": minimized,
        "cloaked": cloaked,
        "topmost": topmost,
        "clickthrough": clickthrough,
        "webview": webview,
        "in_work_percent": in_work_percent,
        "above": above,
        "monitors": monitors,
    }


def _now():
    return datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")


def _describe_above(item, width=40):
    return f"{item['title'][:width]!r} [{item['process']}]"


def report(result, emit):
    emit(f"[{_now()}] {result['verdict']}: {result['detail']}")
    if "hwnd" in result:
        emit(
            f"  pid={result['pid']} hwnd=0x{result['hwnd']:X} rect={result['rect']} "
            f"size={result['size']} in_work={result['in_work_percent']}%"
        )
        emit(
            f"  visible={result['visible']} minimized={result['minimized']} cloaked={result['cloaked']} "
            f"topmost={result['topmost']} clickthrough={result['clickthrough']} "
            f"webview_windows={result['webview']}"
        )
    for monitor in result["monitors"]:
        emit(f"  monitor rc={monitor['monitor']} work={monitor['work']} primary={monitor['primary']}")
    if "hwnd" in result:
        if not result["above"]:
            emit("  above: 无")
        for item in result["above"]:
            emit(
                f"  above: {_describe_above(item)} rect={item['rect']} "
                f"topmost={item['topmost']} clickthrough={item['clickthrough']}"
            )


def _summary(result):
    parts = [f"[{_now()}] {result['verdict']}"]
    if "hwnd" in result:
        parts.append(
            f"pid={result['pid']} rect={result['rect']} topmost={result['topmost']} "
            f"cloaked={result['cloaked']} webview_windows={result['webview']}"
        )
    if result.get("above"):
        parts.append("above=" + ", ".join(_describe_above(item, 30) for item in result["above"][:3]))
    parts.append(result["detail"])
    return " | ".join(parts)


def _state_key(result):
    # 不含 rect：轮播切换 provider 时悬浮窗宽度每隔几秒就变，带上它会刷屏。
    # 位置问题由 OFFSCREEN 判定和 in_work_percent 体现。
    return (
        result["verdict"],
        result.get("pid"),
        result.get("hwnd"),
        result.get("topmost"),
        result.get("cloaked"),
        result.get("in_work_percent"),
        tuple(item["hwnd"] for item in result.get("above", [])),
    )


def watch(seconds, interval, emit):
    deadline = time.monotonic() + seconds if seconds > 0 else None
    emit(f"[{_now()}] 开始监视，状态变化时输出一行；Ctrl+C 结束。")
    last_key = None
    last_monitors = None
    hwnd = None
    while True:
        result = probe(hwnd)
        hwnd = result.get("hwnd")
        monitors = tuple((m["monitor"], m["work"], m["primary"]) for m in result["monitors"])
        if monitors != last_monitors:
            layout = "; ".join(
                f"rc={rc} work={work}{' primary' if primary else ''}" for rc, work, primary in monitors
            )
            emit(f"[{_now()}] 显示器布局：{layout or '无'}")
            last_monitors = monitors
        key = _state_key(result)
        if key != last_key:
            emit(_summary(result))
            last_key = key
        if deadline is not None and time.monotonic() >= deadline:
            break
        time.sleep(interval)


def main():
    # 先于 argparse：--help 的中文也要按 UTF-8 输出。
    try:
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass

    parser = argparse.ArgumentParser(description="诊断 Codex Usage Overlay 悬浮窗为什么看不见")
    parser.add_argument("--watch", action="store_true", help="持续监视，只在状态变化时输出一行")
    parser.add_argument(
        "--seconds", type=float, default=0, help="监视时长（秒），0 表示一直跑到 Ctrl+C；大于 0 时自动进入 --watch"
    )
    parser.add_argument("--interval", type=float, default=0.5, help="采样间隔（秒），最小 0.1")
    parser.add_argument(
        "--log", metavar="FILE", help="同时把输出追加写入 FILE（UTF-8）；比 PowerShell 的 > 重定向可靠，不会乱码"
    )
    args = parser.parse_args()

    log = open(args.log, "a", encoding="utf-8") if args.log else None

    def emit(line):
        print(line, flush=True)
        if log:
            log.write(line + "\n")
            log.flush()

    try:
        if not enable_dpi_awareness():
            emit("注意：未能切换到 Per-Monitor DPI 感知，下面的坐标可能被系统按缩放比例虚拟化。")
        if args.watch or args.seconds > 0:
            watch(args.seconds, max(args.interval, 0.1), emit)
        else:
            report(probe(), emit)
    except KeyboardInterrupt:
        emit(f"[{_now()}] 已停止监视")
    finally:
        if log:
            log.close()


if __name__ == "__main__":
    main()
