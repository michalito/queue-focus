// GNOME runtime adapter for the connection module.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {QueueConnection} from './connection.js';

const BUS_NAME = 'org.queuefocus.QueueFocus';
const OBJ_PATH = '/org/queuefocus/QueueFocus';
// The part of org.queuefocus.QueueFocus1 this indicator uses.
const IFACE_XML = `
<node>
  <interface name="org.queuefocus.QueueFocus1">
    <method name="GetState"><arg type="s" name="json" direction="out"/></method>
    <method name="Add"><arg type="s" name="text" direction="in"/><arg type="s" name="bucket" direction="in"/><arg type="t" name="id" direction="out"/></method>
    <method name="CompleteCurrent"><arg type="t" name="id" direction="out"/><arg type="s" name="title" direction="out"/></method>
    <method name="UndoComplete"><arg type="t" name="id" direction="in"/><arg type="b" name="undone" direction="out"/></method>
    <method name="Promote"><arg type="t" name="id" direction="in"/></method>
    <method name="Show"><arg type="s" name="view" direction="in"/></method>
    <method name="GetSettings"><arg type="s" name="json" direction="out"/></method>
    <signal name="Changed"><arg type="s" name="json"/></signal>
    <signal name="SettingsChanged"><arg type="s" name="json"/></signal>
    <signal name="Flash"><arg type="s" name="json"/></signal>
    <signal name="DurabilityWarning"><arg type="s" name="message"/></signal>
    <signal name="Stopping"/>
  </interface>
</node>`;
const QueueFocusProxy = Gio.DBusProxy.makeProxyWrapper(IFACE_XML);


class DBusRuntime {
    start(events) {
        this._destroyed = false;
        this._proxy = new QueueFocusProxy(Gio.DBus.session, BUS_NAME, OBJ_PATH, (_proxy, error) => {
            if (!this._destroyed) events.ready(error);
        });
        this._signals = ['Changed', 'SettingsChanged', 'Flash', 'DurabilityWarning', 'Stopping']
            .map(name => this._proxy.connectSignal(name, (_proxy, sender, args) => {
                events.signal(sender, name, args[0]);
            }));
        this._ownerId = this._proxy.connect('notify::g-name-owner', () => events.ownerChanged(this.owner));
    }

    get owner() { return this._proxy?.g_name_owner ?? null; }

    call(method, args, done) {
        try {
            this._proxy[`${method}Remote`](...args, done);
        } catch (error) {
            done(null, error);
        }
    }

    schedule(delay, callback) {
        return GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
            callback();
            return GLib.SOURCE_REMOVE;
        });
    }

    cancel(id) { GLib.source_remove(id); }

    isUnavailable(error) {
        return error.matches?.(Gio.DBusError, Gio.DBusError.SERVICE_UNKNOWN) ?? false;
    }

    errorMessage(error) {
        const interrupted = [Gio.DBusError.NO_REPLY, Gio.DBusError.TIMEOUT,
            Gio.DBusError.TIMED_OUT, Gio.DBusError.DISCONNECTED, Gio.DBusError.NAME_HAS_NO_OWNER]
            .some(code => error.matches?.(Gio.DBusError, code)) ||
            error.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.TIMED_OUT);
        if (error instanceof GLib.Error) Gio.DBusError.strip_remote_error(error);
        return interrupted
            ? `${error.message}. The result could not be confirmed. Check the queue before trying again.`
            : error.message;
    }

    destroy() {
        this._destroyed = true;
        for (const id of this._signals) this._proxy.disconnectSignal(id);
        this._proxy.disconnect(this._ownerId);
        this._proxy = null;
    }
}

export function connectQueue(observers) {
    return new QueueConnection(new DBusRuntime(), observers);
}
