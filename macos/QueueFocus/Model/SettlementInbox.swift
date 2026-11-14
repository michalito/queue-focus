/// The sync server's receipts as the model hears them.
///
/// A receipt comes two ways: once as an event on the engine's tick, and
/// again on every refresh while it is the last one kept in the sync status.
/// Either can come first, and the event can be lost. The inbox tells each
/// receipt once, however it came, and holds a receipt for an add field that
/// waits for it, for as long as that field is there.
///
/// A refusal is told on the message line during the run it arrives in. The
/// lasting record of it is the recovery item the core keeps, which the sync
/// line counts and Settings lists, so the receipt the status keeps from
/// before launch counts as told.
struct SettlementInbox {
    /// How many event receipts it remembers having told. Only one change to
    /// Now is in flight at a time, so this is many times what can arrive
    /// between a receipt's event and its turn in the status.
    static let remembered = 64

    /// Receipts already told, oldest first.
    private var told: [String] = []
    /// The receipt the status keeps. It can stay there for days, while
    /// queued edits' receipts pass through `told`, so it is remembered here
    /// until the status replaces it.
    private var statusReceipt: String?
    /// The fields waiting, by command. Weak: a field that goes stops waiting.
    private var waiting: [String: Waiter] = [:]
    /// Receipts for fields still waiting, by command.
    private(set) var held: [String: CommandSettlement] = [:]

    private struct Waiter {
        weak var field: AnyObject?
    }

    /// Hear a receipt. Returns whether it is news, to be told: a receipt
    /// already heard either way is not.
    mutating func hear(_ settlement: CommandSettlement) -> Bool {
        let command = settlement.commandId
        guard !told.contains(command) else { return false }
        told.append(command)
        if told.count > Self.remembered { told.removeFirst(told.count - Self.remembered) }
        if waiting[command]?.field != nil { held[command] = settlement }
        return true
    }

    /// Hear the receipt the status keeps, as every refresh does. Returns
    /// whether it is news: not while it is the one the status kept before.
    mutating func hearInStatus(_ settlement: CommandSettlement) -> Bool {
        guard settlement.commandId != statusReceipt else { return false }
        statusReceipt = settlement.commandId
        return hear(settlement)
    }

    /// Count the status's receipt as told without telling it: the one kept
    /// from before the app started.
    mutating func alreadyTold(_ settlement: CommandSettlement) {
        _ = hearInStatus(settlement)
    }

    /// Hold the receipt for `command` for `field`. `latest` is the receipt
    /// the status keeps now, in case this one has come already.
    mutating func wait(for command: String, by field: AnyObject, latest: CommandSettlement?) {
        waiting[command] = Waiter(field: field)
        if let latest, latest.commandId == command { held[command] = latest }
    }

    /// The receipt for `command`, once it has come. Taking it ends the wait.
    mutating func take(_ command: String) -> CommandSettlement? {
        guard let settlement = held.removeValue(forKey: command) else { return nil }
        waiting[command] = nil
        return settlement
    }

    /// Whether a field that was waiting has gone.
    var hasAbandoned: Bool {
        waiting.values.contains { $0.field == nil }
    }

    /// Forget the fields that have gone, and their receipts.
    mutating func dropAbandoned() {
        for (command, waiter) in waiting where waiter.field == nil {
            waiting[command] = nil
            held[command] = nil
        }
    }
}
