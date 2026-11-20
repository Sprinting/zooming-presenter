//! The Mac's own HTTP transport for web video links (NSURLSession), so a
//! link is checked and downloaded the way Safari and AVPlayer would reach
//! it: the system's proxies, VPNs and App Transport Security. What the
//! answers mean is decided in `link.rs`, the same on every platform.
//!
//! * An ephemeral session: no cookies sent or kept, no cache, no saved
//!   passwords.
//! * A redirect from https to anything else is refused, and so is a sixth
//!   redirect (a loop).
//! * The delegate runs on its own serial queue and hands the answer over a
//!   bounded channel, so a fast server waits for a slow disk rather than
//!   filling memory; the reader gives up when nothing arrives in time.
//! * Dropping the answer cancels the request; the session is invalidated
//!   as soon as its one task starts, so it frees itself when that ends.
use std::io::Read;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{
    NSData, NSError, NSHTTPCookieAcceptPolicy, NSHTTPURLResponse, NSMutableURLRequest,
    NSOperationQueue, NSString, NSURL, NSURLRequest, NSURLRequestCachePolicy, NSURLResponse,
    NSURLSession, NSURLSessionConfiguration, NSURLSessionDataDelegate, NSURLSessionDataTask,
    NSURLSessionDelegate, NSURLSessionResponseDisposition, NSURLSessionTask,
    NSURLSessionTaskDelegate,
};

use crate::link::{Head, Limits, LinkError, Opened};

/// Redirects followed before a link counts as unreachable.
const MAX_REDIRECTS: u32 = 5;
/// Chunks the delegate may run ahead of the reader (NSURLSession hands
/// over at most a few hundred KB at a time).
const QUEUE: usize = 16;
/// NSURLErrorAppTransportSecurityRequiresSecureConnection: ATS refused
/// plain http (a redirect we didn't see, or an http subresource).
const ATS_REFUSED: isize = -1022;

enum Event {
    Head(Head),
    Data(Vec<u8>),
    Done(Result<(), LinkError>),
}

struct Ivars {
    /// Taken by the first terminal event; later ones are dropped.
    tx: Mutex<Option<SyncSender<Event>>>,
    hops: AtomicU32,
    start_https: bool,
}

impl Ivars {
    fn send(&self, e: Event) -> bool {
        let tx = match self.tx.lock() {
            Ok(g) => g.clone(),
            Err(_) => None,
        };
        // Outside the lock: a full queue blocks this (the delegate's own)
        // thread until the reader catches up or goes away.
        tx.is_some_and(|tx| tx.send(e).is_ok())
    }
    fn finish(&self, r: Result<(), LinkError>) {
        let tx = self.tx.lock().ok().and_then(|mut g| g.take());
        if let Some(tx) = tx {
            let _ = tx.send(Event::Done(r));
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements; the class adds
    // no Drop and its ivars are Send + Sync.
    #[unsafe(super(NSObject))]
    #[name = "ZebraWebLinkDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSURLSessionDelegate for Delegate {}

    unsafe impl NSURLSessionTaskDelegate for Delegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn will_redirect(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            _response: &NSHTTPURLResponse,
            request: &NSURLRequest,
            done: &DynBlock<dyn Fn(*mut NSURLRequest)>,
        ) {
            let iv = self.ivars();
            let to = request.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
            let https = to.to_ascii_lowercase().starts_with("https://");
            let hops = iv.hops.fetch_add(1, Ordering::Relaxed) + 1;
            let refuse = if iv.start_https && !https {
                Some(LinkError::Http)
            } else if hops > MAX_REDIRECTS {
                Some(LinkError::Unreachable)
            } else {
                None
            };
            match refuse {
                Some(e) => {
                    iv.finish(Err(e));
                    done.call((std::ptr::null_mut(),));
                    task.cancel();
                }
                None => done.call((request as *const NSURLRequest as *mut NSURLRequest,)),
            }
        }

        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(&self, _session: &NSURLSession, _task: &NSURLSessionTask, error: Option<&NSError>) {
            let r = match error {
                None => Ok(()),
                Some(e) if e.code() == ATS_REFUSED => Err(LinkError::Http),
                // Cancelled, offline, timed out, no such host, a bad
                // certificate: all "couldn't reach".
                Some(_) => Err(LinkError::Unreachable),
            };
            self.ivars().finish(r);
        }
    }

    unsafe impl NSURLSessionDataDelegate for Delegate {
        #[unsafe(method(URLSession:dataTask:didReceiveResponse:completionHandler:))]
        fn did_receive_response(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDataTask,
            response: &NSURLResponse,
            done: &DynBlock<dyn Fn(NSURLSessionResponseDisposition)>,
        ) {
            let iv = self.ivars();
            let status = response.downcast_ref::<NSHTTPURLResponse>().map(|h| h.statusCode()).unwrap_or(0);
            let status = u16::try_from(status).unwrap_or(0);
            let length = response.expectedContentLength();
            let head = Head {
                status,
                content_type: response.MIMEType().map(|m| m.to_string().to_ascii_lowercase()).unwrap_or_default(),
                final_url: response.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default(),
                redirected: iv.hops.load(Ordering::Relaxed) > 0,
                length: u64::try_from(length).ok(),
            };
            if iv.send(Event::Head(head)) {
                done.call((NSURLSessionResponseDisposition::Allow,));
            } else {
                done.call((NSURLSessionResponseDisposition::Cancel,));
                task.cancel();
            }
        }

        #[unsafe(method(URLSession:dataTask:didReceiveData:))]
        fn did_receive_data(&self, _session: &NSURLSession, task: &NSURLSessionDataTask, data: &NSData) {
            if !self.ivars().send(Event::Data(data.to_vec())) {
                // Nobody reads any more.
                task.cancel();
            }
        }
    }
);

impl Delegate {
    fn new(tx: SyncSender<Event>, start_https: bool) -> Retained<Delegate> {
        let this = Delegate::alloc().set_ivars(Ivars { tx: Mutex::new(Some(tx)), hops: AtomicU32::new(0), start_https });
        // SAFETY: NSObject's designated initialiser on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

/// An answer's body as it arrives. Dropping it cancels the request.
struct Body {
    rx: Receiver<Event>,
    task: Retained<NSURLSessionDataTask>,
    limits: Limits,
    chunk: Vec<u8>,
    at: usize,
    ended: bool,
}

impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        loop {
            if self.at < self.chunk.len() {
                let n = out.len().min(self.chunk.len() - self.at);
                out[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            if self.ended || out.is_empty() {
                return Ok(0);
            }
            let wait = self.limits.left().map_err(|_| std::io::Error::from(std::io::ErrorKind::TimedOut))?;
            match self.rx.recv_timeout(wait) {
                Ok(Event::Data(d)) => {
                    self.chunk = d;
                    self.at = 0;
                }
                Ok(Event::Head(_)) => {}
                Ok(Event::Done(Ok(()))) => self.ended = true,
                Ok(Event::Done(Err(_))) | Err(RecvTimeoutError::Disconnected) => {
                    self.ended = true;
                    return Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.task.cancel();
                    self.ended = true;
                    return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
                }
            }
        }
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        self.task.cancel();
    }
}

/// GET `url` and wait for its status and headers (within `limits`).
pub(crate) fn open(url: &str, range: Option<u64>, limits: Limits) -> Result<Opened, LinkError> {
    let left = limits.left()?;
    let u = NSURL::URLWithString(&NSString::from_str(url.trim())).ok_or(LinkError::NotLink)?;
    let start_https = url.trim().to_ascii_lowercase().starts_with("https://");
    let (tx, rx) = sync_channel(QUEUE);
    let delegate = Delegate::new(tx, start_https);

    let cfg = NSURLSessionConfiguration::ephemeralSessionConfiguration();
    // Nothing of the user's goes with the request, and nothing is kept.
    cfg.setHTTPCookieStorage(None);
    cfg.setHTTPShouldSetCookies(false);
    cfg.setHTTPCookieAcceptPolicy(NSHTTPCookieAcceptPolicy::Never);
    cfg.setURLCache(None);
    cfg.setURLCredentialStorage(None);
    cfg.setRequestCachePolicy(NSURLRequestCachePolicy::ReloadIgnoringLocalCacheData);
    cfg.setWaitsForConnectivity(false);
    // Between two packets: the check's time left, or a download's stall
    // time. A download has no overall limit beyond the system's.
    cfg.setTimeoutIntervalForRequest(limits.idle.as_secs_f64().max(1.0));
    if limits.deadline.is_some() {
        cfg.setTimeoutIntervalForResource(left.as_secs_f64().max(1.0));
    }

    let req = NSMutableURLRequest::requestWithURL(&u);
    if let Some(max) = range {
        req.setValue_forHTTPHeaderField(Some(&NSString::from_str(&format!("bytes=0-{}", max.saturating_sub(1)))), &NSString::from_str("Range"));
    }
    req.setValue_forHTTPHeaderField(Some(&NSString::from_str("*/*")), &NSString::from_str("Accept"));

    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(1);
    // SAFETY: a private serial operation queue for the delegate, which
    // only touches its own ivars (Send + Sync) from there.
    let session = unsafe {
        NSURLSession::sessionWithConfiguration_delegate_delegateQueue(&cfg, Some(ProtocolObject::from_ref(&*delegate)), Some(&queue))
    };
    let task = session.dataTaskWithRequest(&req);
    task.resume();
    // The session lets go of its delegate once its one task is over.
    session.finishTasksAndInvalidate();

    let body = Body { rx, task, limits, chunk: vec![], at: 0, ended: false };
    let first = body.rx.recv_timeout(left);
    match first {
        Ok(Event::Head(head)) => Ok(Opened { head, body: Box::new(body) }),
        Ok(Event::Done(Err(e))) => Err(e),
        Ok(_) | Err(_) => Err(LinkError::Unreachable),
    }
}
