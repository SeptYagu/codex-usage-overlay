export const invoke = async (cmd) => cmd === 'is_installed_version' ? true : cmd === 'get_notification_status' ? 'enabled' : null;
export const listen = async () => () => { };
export class LogicalSize {
    constructor(width, height) { this.width = width; this.height = height; }
}
export const getCurrentWindow = () => ({ scaleFactor: async () => window.devicePixelRatio, onScaleChanged: async () => () => { }, setSize: async () => { }, setTitle: async () => { }, close: async () => { } });
export const addPluginListener = async () => ({ unregister: async () => { } });
