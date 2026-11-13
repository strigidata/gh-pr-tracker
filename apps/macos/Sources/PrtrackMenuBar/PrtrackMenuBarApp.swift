import AppKit
import PrtrackFFI
import SwiftUI

@main
struct PrtrackMenuBarApp: App {
    @State private var model = BoardModel()

    init() {
        // No bundle, so no LSUIElement.
        NSApplication.shared.setActivationPolicy(.accessory)
    }

    var body: some Scene {
        MenuBarExtra {
            PopoverView(model: model)
        } label: {
            BadgeLabel(badge: model.badge)
        }
        .menuBarExtraStyle(.window)
    }
}

struct BadgeLabel: View {
    let badge: BoardModel.Badge

    var body: some View {
        HStack(spacing: 4) {
            switch badge {
            case .unknown:
                // Unknown is "?", never "0".
                Image(systemName: "arrow.triangle.pull")
                Text("?")
            case .current(let count):
                Image(systemName: "arrow.triangle.pull")
                Text("\(count)")
            case .stale(let count):
                // The last good count, after a fetch that failed.
                Image(systemName: "exclamationmark.triangle")
                Text("\(count)")
            }
        }
    }
}

struct PopoverView: View {
    let model: BoardModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            header
            Divider()
            content
            Divider()
            HStack {
                Button("Refresh") { Task { await model.refresh() } }
                    .disabled(model.isRefreshing)
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
            }
        }
        .padding(12)
        .frame(width: 460)
        .task { await model.refresh() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(model.repo.isEmpty ? "(no repository)" : model.repo).font(.headline)
                Spacer()
                if let viewer = currentBoard?.viewer { Text(viewer).foregroundStyle(.secondary) }
            }
            HStack(spacing: 6) {
                if case .loaded(_, let at) = model.state {
                    Text("Fetched \(Text(at, style: .relative)) ago")
                }
                // Whatever is on screen while a fetch runs, board or failure,
                // is marked as being replaced. `.loading` says so itself.
                if model.isRefreshing, !model.isLoading {
                    ProgressView().controlSize(.small)
                    Text("Refreshing...")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }

    private var currentBoard: PopoverBoard? {
        switch model.state {
        case .loading: nil
        case .loaded(let board, _): board
        case .failed(_, let lastGood, _): lastGood
        }
    }

    @ViewBuilder private var content: some View {
        switch model.state {
        case .loading:
            Text("Loading...").foregroundStyle(.secondary)
        case .loaded(let board, _):
            boardView(board, ifEmpty: "Nothing open: the fetch succeeded and the board is empty.")
        case .failed(let message, let lastGood, let at):
            VStack(alignment: .leading, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Could not refresh").bold()
                    Text(message)
                    if let at {
                        Text("Showing the board fetched \(Text(at, style: .relative)) ago")
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.red.opacity(0.2), in: RoundedRectangle(cornerRadius: 6))
                if let lastGood {
                    boardView(lastGood, ifEmpty: "Nothing was open at the last successful fetch.")
                }
            }
        }
    }

    /// `ifEmpty` is said when the core counted no pull requests; only a
    /// current board may claim the repository is empty now.
    @ViewBuilder private func boardView(_ board: PopoverBoard, ifEmpty: String) -> some View {
        // The record's own order; Rust decided it.
        let sections: [(String, [PopoverRow])] = [
            ("Needs your review", board.needsYourReview),
            ("Unclaimed", board.unclaimed),
            ("You reviewed", board.youReviewed),
            ("Your pull requests", board.yours),
            ("Your drafts", board.yourDrafts),
            ("Other drafts", board.otherDrafts),
        ]
        let shown = sections.filter { !$0.1.isEmpty }
        // Not a count of anything: a check that every row the core counted
        // arrived in a section this app draws, e.g. after Rust adds a section.
        let drawn = sections.reduce(0) { $0 + $1.1.count }
        if drawn != Int(board.total) {
            Text("The board counts \(board.total) pull requests but \(drawn) rows arrived; this app may be out of date.")
                .foregroundStyle(.orange)
        }
        if board.total == 0 {
            Text(ifEmpty).foregroundStyle(.secondary)
        }
        if !shown.isEmpty {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(shown, id: \.0) { title, rows in
                        Text(title).font(.subheadline).bold()
                        ForEach(rows, id: \.url) { row in RowView(row: row) }
                    }
                }
            }
            .frame(maxHeight: 420)
        }
    }
}

struct RowView: View {
    let row: PopoverRow

    var body: some View {
        Button {
            if let url = URL(string: row.url) { NSWorkspace.shared.open(url) }
        } label: {
            VStack(alignment: .leading, spacing: 1) {
                Text("#\(row.number) \(row.title)").lineLimit(1)
                Text([wording(for: row), row.author].compactMap { $0 }.joined(separator: " - "))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.leading, CGFloat(row.depth) * 16)
    }
}
