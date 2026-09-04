// A module resolver that points GNOME Shell's import schemes at the stubs, so
// the extension's own source can be loaded by node exactly as it ships.

const STUBS = JSON.stringify(new URL('./gi-stubs.mjs', import.meta.url).href);

export function resolve(specifier, context, next) {
    const namespace = specifier.startsWith('gi://') && specifier.slice('gi://'.length);
    if (namespace) {
        return {
            url: `data:text/javascript,export {${namespace} as default} from ${encodeURIComponent(STUBS)}`,
            shortCircuit: true,
        };
    }
    if (specifier === 'resource:///org/gnome/shell/ui/main.js') {
        return {
            url: `data:text/javascript,export {layoutManager, uiGroup} from ${encodeURIComponent(STUBS)}`,
            shortCircuit: true,
        };
    }
    return next(specifier, context);
}
