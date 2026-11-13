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
        case failed(Failure, lastGood: PopoverBoard?, at: Date?)
    }

    /// Why the board could not be refreshed.
    struct Failure {
        /// What to do about it, when this app knows; said above `message`.
        let hint: String?
        /// What failed, and every cause beneath it.
        let message: String

        init(hint: String?, message: String) {
            self.hint = hint
            self.message = message
        }

        init(_ error: FetchError) {
            // The generated `errorDescription` is `String(reflecting:)`, which
            // prints the Swift case around the text; show the text itself.
            // No `default:`, so a case Rust adds fails to compile here.
            let message =
                switch error {
                case .NoToken(let message), .Unauthorized(let message),
                    .RateLimited(let message), .RepositoryUnavailable(let message),
                    .Network(let message), .Unreadable(let message):
                    message
                }
            self.init(hint: failureHint(for: error), message: message)
        }
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
            let failure = Failure(hint: nil, message: "PRTRACK_REPO is not set (expected owner/name).")
            state = .failed(failure, lastGood: nil, at: nil)
            return
        }
        let started = Date()
        do {
            let board = try await fetchBoard(repo: repo)
            state = .loaded(board, at: started)
        } catch let error as FetchError {
            fail(Failure(error))
        } catch {
            // Not one of Rust's answers: a failure in the generated glue.
            fail(Failure(hint: nil, message: error.localizedDescription))
        }
    }

    /// Shows `failure`, keeping whatever board was last fetched.
    private func fail(_ failure: Failure) {
        switch state {
        case .loaded(let board, let at):
            state = .failed(failure, lastGood: board, at: at)
        case .failed(_, let lastGood, let at):
            state = .failed(failure, lastGood: lastGood, at: at)
        case .loading:
            state = .failed(failure, lastGood: nil, at: nil)
        }
    }
}
