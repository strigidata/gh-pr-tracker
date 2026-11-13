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
            HStack(spacing: 4) {
                Image(systemName: "arrow.triangle.pull")
                // Unknown is "?", never "0".
                Text(model.badge.map(String.init) ?? "?")
            }
        }
        .menuBarExtraStyle(.window)
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
                Spacer()
                Button("Quit") { NSApplication.shared.terminate(nil) }
            }
        }
        .padding(12)
        .frame(width: 460)
        .task { await model.refresh() }
    }

    private var header: some View {
        HStack {
            Text(model.repo.isEmpty ? "(no repository)" : model.repo).font(.headline)
            Spacer()
            if let viewer = currentBoard?.viewer { Text(viewer).foregroundStyle(.secondary) }
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
            boardView(board)
        case .failed(let message, let lastGood, let at):
            VStack(alignment: .leading, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Could not refresh").bold()
                    Text(message)
                    if let at {
                        Text("Showing the board from \(at.formatted(.relative(presentation: .numeric)))")
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.red.opacity(0.2), in: RoundedRectangle(cornerRadius: 6))
                if let lastGood { boardView(lastGood) }
            }
        }
    }

    @ViewBuilder private func boardView(_ board: PopoverBoard) -> some View {
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
        if shown.isEmpty {
            Text("Nothing open: the fetch succeeded and the board is empty.")
                .foregroundStyle(.secondary)
        } else {
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
                Text([wording(for: row.reason), row.author].compactMap { $0 }.joined(separator: " - "))
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
