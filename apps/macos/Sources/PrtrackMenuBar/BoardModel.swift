import Foundation
import Observation
import PrtrackFFI

@MainActor
@Observable
final class BoardModel {
    enum State {
        case loading
        case loaded(PopoverBoard, at: Date)
        /// `at` is when `lastGood` was fetched.
        case failed(String, lastGood: PopoverBoard?, at: Date?)
    }

    private(set) var state: State = .loading
    let repo: String

    init(repo: String = ProcessInfo.processInfo.environment["PRTRACK_REPO"] ?? "") {
        self.repo = repo
    }

    /// The count the menu bar shows. Nil means "unknown", never "zero".
    var badge: Int? {
        switch state {
        case .loading: nil
        case .loaded(let board, _): Int(board.reviewableNow)
        case .failed(_, let lastGood, _): lastGood.map { Int($0.reviewableNow) }
        }
    }

    func refresh() async {
        guard !repo.isEmpty else {
            state = .failed("PRTRACK_REPO is not set (expected owner/name).", lastGood: nil, at: nil)
            return
        }
        do {
            let board = try await fetchBoard(repo: repo)
            if Task.isCancelled { return }
            state = .loaded(board, at: Date())
        } catch is CancellationError {
            return
        } catch {
            if Task.isCancelled { return }
            let message = error.localizedDescription
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
