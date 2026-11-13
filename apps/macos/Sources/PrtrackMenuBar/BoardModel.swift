import Foundation
import Observation
import PrtrackFFI

@MainActor
@Observable
final class BoardModel {
    enum State {
        case loading
        /// `at` is when the fetch that produced `board` started.
        case loaded(PopoverBoard, at: Date)
        /// `at` is when the fetch that produced `lastGood` started.
        case failed(String, lastGood: PopoverBoard?, at: Date?)
    }

    /// What the menu bar shows. Unknown is never zero, and a count from before
    /// a failed fetch is never shown as current. A successful count's age is
    /// not tracked: nothing refreshes until the popover opens, so `.current`
    /// can be hours old. That waits for polling.
    enum Badge: Equatable {
        case unknown
        case current(Int)
        case stale(Int)
    }

    private(set) var state: State = .loading
    /// True while a fetch runs, including one over a board already shown.
    private(set) var isRefreshing = false

    /// True before the first fetch has answered, when there is nothing to mark
    /// as being replaced.
    var isLoading: Bool {
        if case .loading = state { true } else { false }
    }
    let repo: String

    /// The one fetch in flight. Owned here, not by a view, so a view going
    /// away does not abandon it half-way.
    private var inFlight: Task<Void, Never>?

    init(repo: String = ProcessInfo.processInfo.environment["PRTRACK_REPO"] ?? "") {
        self.repo = repo
    }

    var badge: Badge {
        switch state {
        case .loading: .unknown
        case .loaded(let board, _): .current(Int(board.reviewableNow))
        case .failed(_, let lastGood, _): lastGood.map { .stale(Int($0.reviewableNow)) } ?? .unknown
        }
    }

    /// Fetches the board, or joins the fetch already running.
    func refresh() async {
        if let inFlight {
            await inFlight.value
            return
        }
        // Inherits the main actor, so it cannot start before `inFlight` is set.
        let fetch = Task {
            await self.fetch()
            self.inFlight = nil
            self.isRefreshing = false
        }
        inFlight = fetch
        isRefreshing = true
        await fetch.value
    }

    // No cancellation checks: nothing cancels this task, which no view owns,
    // and cancelling it would not stop the Rust future anyway.
    private func fetch() async {
        guard !repo.isEmpty else {
            state = .failed("PRTRACK_REPO is not set (expected owner/name).", lastGood: nil, at: nil)
            return
        }
        let started = Date()
        do {
            let board = try await fetchBoard(repo: repo)
            state = .loaded(board, at: started)
        } catch {
            // The generated `errorDescription` is `String(reflecting:)`, which
            // prints the Swift case around the text; show the text itself.
            let message =
                switch error {
                case FetchError.Failed(let message): message
                default: error.localizedDescription
                }
            switch state {
            case .loaded(let board, let at):
                state = .failed(message, lastGood: board, at: at)
            case .failed(_, let lastGood, let at):
                state = .failed(message, lastGood: lastGood, at: at)
            case .loading:
                state = .failed(message, lastGood: nil, at: nil)
            }
        }
    }
}
