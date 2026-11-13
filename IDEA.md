# Quick Apps

A new cross platform UI system that is similar to the web but way better.

## Runtime

Wasm at the core, but with a simple rendering engine.

## Rendering

System of nested grids with a large collection of included components (which are just wasm files).

Each cell has a set of constraints and a priority, the rendering system then solves it.

There is no CSS, components get any custom styles from the "context" that is passed to them (this context is designed to be able to be passed down easily, kind of like how css cascades)

## IO

Each app that is run gets a folder by default.

Apps can send messages to each other. 

Apps can request a file picker, they can also request access to any folder.

Apps must request access to the internet.

All of these things must be prompts that happen "outside" the app.

Apps must request access to use more than 50mb of memory or more than 50% of the system cpu.

## Compatibility

The apps get limited information about the environment
- Screen size
- Is mobile

The runtime should be able to run on any platform.

## Hosting

Apps can be hosted over HTTP(S). The app is stored as a zip bundle with a
manifest stored next to it. The app is cached on first download. On every open,
the runtime checks if the manifest has an updated version and asks if the user
would like to download it.

