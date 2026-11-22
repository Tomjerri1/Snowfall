//! Transparent, click-through, always-on-top overlay window spanning all screens.
//!
//! A borderless `NSWindow` with a clear background whose layer shows one `CGImage` per frame.
//! The buffer is premultiplied BGRA (`kCGImageAlphaPremultipliedFirst` + 32-bit little endian), like on the other platforms.
//! It has one pixel per point, so on Retina displays the snow is softly upscaled by the system.

use super::screens::Layout;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSEventMask,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_foundation::{NSDefaultRunLoopMode, NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CALayer, CATransaction};
use std::ffi::c_void;

/// Above the menu bar, the Dock and menus (`NSScreenSaverWindowLevel`).
const OVERLAY_LEVEL: isize = 1000;

pub struct Overlay {
    app: Retained<NSApplication>,
    window: Retained<NSWindow>,
    layer: Retained<CALayer>,
    /// Draws into `buf`; kept alive for as long as `buf` is.
    context: CFRetained<CGContext>,
    /// Premultiplied BGRA, top-down, `width * height * 4` bytes. Never reallocated: `context` points into it.
    buf: Vec<u8>,
    width: i32,
    height: i32,
    origin: (i32, i32),
    visible: bool,
}

impl Overlay {
    pub fn new(mtm: MainThreadMarker, layout: &Layout) -> Result<Self, String> {
        let (left, top, right, bottom) = layout.bounds;
        let (width, height) = (right - left, bottom - top);
        if width <= 0 || height <= 0 {
            return Err("the virtual screen is empty".into());
        }

        let app = NSApplication::sharedApplication(mtm);
        // No Dock icon and no menu bar: the app lives in the status bar.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();

        // Cocoa window frames are measured from the bottom of the primary screen.
        let frame = NSRect::new(
            NSPoint::new(left as f64, layout.quartz_y(bottom as f64)),
            NSSize::new(width as f64, height as f64),
        );
        // SAFETY: a plain borderless window created on the main thread with valid arguments.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: the window is owned by us and never closed by AppKit.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setOpaque(false);
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setHasShadow(false);
        window.setIgnoresMouseEvents(true);
        window.setLevel(OVERLAY_LEVEL);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );

        let view = window
            .contentView()
            .ok_or("the overlay window has no content view")?;
        view.setWantsLayer(true);
        let layer = view.layer().ok_or("the overlay view has no layer")?;
        layer.setOpaque(false);
        layer.setContentsScale(1.0);

        let space = CGColorSpace::new_device_rgb().ok_or("cannot create a color space")?;
        let mut buf = vec![0u8; width as usize * height as usize * 4];
        let bitmap_info =
            CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;
        // SAFETY: `buf` is `width * height * 4` bytes and outlives the context, which is dropped first.
        let context = unsafe {
            CGBitmapContextCreate(
                buf.as_mut_ptr().cast::<c_void>(),
                width as usize,
                height as usize,
                8,
                width as usize * 4,
                Some(&space as &CGColorSpace),
                bitmap_info,
            )
        }
        .ok_or("cannot create a bitmap context")?;

        window.orderFrontRegardless();

        Ok(Self {
            app,
            window,
            layer,
            context,
            buf,
            width,
            height,
            origin: (left, top),
            visible: true,
        })
    }

    /// Virtual screen as (left, top, right, bottom) in Quartz coordinates.
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        (
            self.origin.0,
            self.origin.1,
            self.origin.0 + self.width,
            self.origin.1 + self.height,
        )
    }

    pub fn pixmap_mut(&mut self) -> tiny_skia::PixmapMut<'_> {
        tiny_skia::PixmapMut::from_bytes(&mut self.buf, self.width as u32, self.height as u32)
            .expect("overlay buffer matches its size")
    }

    pub fn set_visible(&mut self, visible: bool) {
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.window.orderFrontRegardless();
        } else {
            self.window.orderOut(None);
        }
    }

    /// Shows the current buffer.
    pub fn present(&mut self) {
        if !self.visible {
            return;
        }
        // The image is a snapshot of the buffer, so the next frame may be drawn right away.
        let Some(image) = CGBitmapContextCreateImage(Some(&self.context)) else {
            return;
        };
        // Without this, Core Animation cross-fades between consecutive frames.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        // SAFETY: a `CGImage` is the expected type of a layer's contents; CoreFoundation objects are Objective-C objects.
        unsafe {
            let contents = &*(&*image as *const CGImage as *const AnyObject);
            self.layer.setContents(Some(contents));
        }
        CATransaction::commit();
    }

    /// Dispatches pending AppKit events (needed by the status item, hotkeys and the window server).
    pub fn pump_events(&mut self) {
        loop {
            // SAFETY: called on the main thread with a valid mode; a nil date returns immediately.
            let event = unsafe {
                self.app.nextEventMatchingMask_untilDate_inMode_dequeue(
                    NSEventMask::Any,
                    None,
                    NSDefaultRunLoopMode,
                    true,
                )
            };
            match event {
                Some(event) => self.app.sendEvent(&event),
                None => break,
            }
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.window.orderOut(None);
        self.window.close();
    }
}
