//! A number and two buttons.
const std = @import("std");
const qa = @import("qa");

var n: i32 = 0;
var label: qa.Node = 0;
var plus: qa.Node = 0;
var reset: qa.Node = 0;

pub fn start() void {
    const root = qa.grid().pad(16).gap(12).root();
    label = qa.text("0").style("title").alignX(.center).addTo(root);
    const row = qa.grid().cols(&.{ qa.fr(1), qa.fr(1) }).gap(8).addTo(root);
    plus = qa.button("+1").variant("primary").addTo(row);
    reset = qa.button("Reset").addTo(row);
}

pub fn event(e: qa.Event) void {
    switch (e) {
        .click => |node| {
            if (node == plus) n += 1;
            if (node == reset) n = 0;
            var buf: [12]u8 = undefined;
            qa.setText(label, std.fmt.bufPrint(&buf, "{d}", .{n}) catch "?");
        },
        else => {},
    }
}

comptime {
    qa.exportApp(@This());
}
