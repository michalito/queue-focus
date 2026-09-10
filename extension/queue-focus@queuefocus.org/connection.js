// Connection lifetime and recovery, independent of GNOME actors. The runtime
// supplies D-Bus and one-shot timers; tests drive the same interface explicitly.
const RETRY_MS = 1500;
const RETRY_MAX_MS = 60000;

export class QueueConnection {
    constructor(runtime, observers) {
        this._runtime = runtime;
        this._observers = observers;
        this._destroyed = false;
        this._owner = null;
        this._stopping = false;
        this._retry = null;
        this._retryMs = RETRY_MS;
        this._reads = 0;
        this._stateReady = false;
        this._settingsReady = false;
        this._stateVersion = 0;
        this._settingsVersion = 0;
        this._pending = new Set();
        runtime.start({
            ready: error => {
                if (this._destroyed) return;
                if (error) {
                    this._observers.state(null);
                    this._scheduleRetry();
                    return;
                }
                if (runtime.owner !== this._owner) this._ownerChanged(runtime.owner);
                else this._refresh();
            },
            ownerChanged: owner => this._ownerChanged(owner),
            signal: (sender, name, value) => this._signal(sender, name, value),
        });
    }

    // Explicit user requests may wake an app that deliberately stopped.
    refresh() {
        if (this._destroyed) return;
        this._stopping = false;
        this._cancelRetry();
        this._refresh();
    }

    request(method, args = [], done = () => {}) {
        if (this._destroyed) return;
        this._stopping = false;
        this._cancelRetry();
        const request = {done};
        this._pending.add(request);
        this._runtime.call(method, args, (result, error) => {
            if (this._destroyed || !this._pending.delete(request)) return;
            // Never replay an action: it may have committed before its reply
            // was lost. Reads reconcile what the app actually persisted.
            const failure = error ? new Error(this._runtime.errorMessage(error)) : null;
            done(result, failure);
            if (this._destroyed || this._stopping) return;
            if (error && this._runtime.isUnavailable(error)) {
                this._idle();
            } else {
                this._refresh();
            }
        });
    }

    _ownerChanged(owner) {
        if (this._destroyed || owner === this._owner) return;
        const previous = this._owner;
        this._owner = owner;
        this._stateReady = this._settingsReady = false;
        ++this._reads;
        // null -> owner is ordinary D-Bus activation. Requests already sent
        // while idle belong to that activation and must be allowed to finish.
        if (previous) {
            const pending = [...this._pending];
            this._pending.clear();
            for (const {done} of pending) {
                if (this._destroyed) return;
                done(null, new Error('The connection changed before the result could be confirmed. Check the queue before trying again.'));
            }
            if (this._destroyed) return;
            this._observers.state(null);
        }
        if (owner) {
            this._stopping = false;
            this._cancelRetry();
            this._refresh();
        } else if (!this._stopping) {
            this._scheduleRetry();
        }
    }

    _signal(sender, name, value) {
        if (this._destroyed || !this._owner || sender !== this._owner) return;
        switch (name) {
        case 'Stopping':
            this._stopping = true;
            ++this._reads;
            this._cancelRetry();
            break;
        case 'Changed':
            ++this._stateVersion;
            this._state(value);
            break;
        case 'SettingsChanged':
            ++this._settingsVersion;
            this._settings(value);
            break;
        case 'Flash': {
            const event = this._parse(value);
            if (event?.title) this._observers.flash(event);
            break;
        }
        case 'DurabilityWarning':
            this._observers.warning(value);
            break;
        }
    }

    _refresh() {
        if (this._destroyed || this._stopping) return;
        this._cancelRetry();
        // Recovery means both snapshots are healthy in this refresh. A
        // working task stream must not reset retries for broken settings.
        this._stateReady = this._settingsReady = false;
        const reads = ++this._reads;
        const stateVersion = this._stateVersion;
        const settingsVersion = this._settingsVersion;
        const current = () => !this._destroyed && !this._stopping && reads === this._reads;
        this._runtime.call('GetSettings', [], (result, error) => {
            if (!current() || settingsVersion !== this._settingsVersion) return;
            if (error) this._scheduleRetry();
            else this._settings(result[0]);
        });
        this._runtime.call('GetState', [], (result, error) => {
            if (!current() || stateVersion !== this._stateVersion) return;
            if (!error) {
                this._state(result[0]);
                return;
            }
            this._observers.state(null);
            if (this._runtime.isUnavailable(error)) this._idle();
            else this._scheduleRetry();
        });
    }

    _parse(json) {
        try {
            const value = JSON.parse(json);
            return value && typeof value === 'object' && !Array.isArray(value) ? value : null;
        } catch (_error) {
            return null;
        }
    }

    _state(json) {
        const state = this._parse(json);
        this._stateReady = state !== null;
        if (state) this._recovered();
        else this._scheduleRetry();
        this._observers.state(state);
    }

    _settings(json) {
        const settings = this._parse(json);
        this._settingsReady = settings !== null;
        if (!settings) {
            this._scheduleRetry();
            return;
        }
        this._recovered();
        this._observers.settings(settings);
    }

    _recovered() {
        if (!this._stateReady || !this._settingsReady) return;
        this._retryMs = RETRY_MS;
        this._cancelRetry();
    }

    _idle() {
        this._stateReady = this._settingsReady = false;
        ++this._reads;
        this._cancelRetry();
        this._observers.state(null);
    }

    _scheduleRetry() {
        if (this._destroyed || this._stopping || this._retry !== null) return;
        this._retry = this._runtime.schedule(this._retryMs, () => {
            this._retry = null;
            this._refresh();
        });
        this._retryMs = Math.min(this._retryMs * 2, RETRY_MAX_MS);
    }

    _cancelRetry() {
        if (this._retry === null) return;
        this._runtime.cancel(this._retry);
        this._retry = null;
    }

    destroy() {
        if (this._destroyed) return;
        this._destroyed = true;
        this._cancelRetry();
        this._pending.clear();
        this._runtime.destroy();
        this._observers = null;
    }
}
