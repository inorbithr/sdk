//! A paged list, walked page after page.

use crate::Error;
use std::collections::VecDeque;
use std::fmt;

use crate::__codegen::Fetch;

/// Every item of a paged list, page after page, from a generated `all_<operation>`.
///
/// It fetches a page only when the items before it are used up, so stopping early (or
/// dropping it) fetches nothing more. It follows the next-page token until a page's token
/// is empty or repeats, and ends after the first error.
///
/// ```no_run
/// # async fn walk(mut digests: inorbithr::Pages<'_, String>) -> Result<(), inorbithr::Error> {
/// while let Some(digest) = digests.next().await {
///     let digest = digest?;
///     println!("{digest}");
/// }
/// # Ok(())
/// # }
/// ```
pub struct Pages<'a, T> {
    fetch: Fetch<'a, T>,
    items: VecDeque<T>,
    token: Option<String>,
    done: bool,
}

impl<T> fmt::Debug for Pages<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pages")
            .field("buffered", &self.items.len())
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl<'a, T> Pages<'a, T> {
    pub(crate) fn new(fetch: Fetch<'a, T>) -> Self {
        Self {
            fetch,
            items: VecDeque::new(),
            token: None,
            done: false,
        }
    }

    /// The next item, fetching the next page when the current one is used up; `None`
    /// after the last item or after an error.
    ///
    /// # Errors
    ///
    /// The error a page's call ended with; the walk ends with it.
    #[allow(clippy::should_implement_trait)] // an async `next`, not `Iterator::next`
    pub async fn next(&mut self) -> Option<Result<T, Error>> {
        loop {
            if let Some(item) = self.items.pop_front() {
                return Some(Ok(item));
            }
            if self.done {
                return None;
            }
            match (self.fetch)(self.token.clone()).await {
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Ok((items, next)) => {
                    self.items.extend(items);
                    if next.is_empty() || self.token.as_deref() == Some(next.as_str()) {
                        self.done = true;
                    } else {
                        self.token = Some(next);
                    }
                }
            }
        }
    }

    /// Every remaining item, in order.
    ///
    /// # Errors
    ///
    /// The first error a page's call ended with.
    pub async fn collect(mut self) -> Result<Vec<T>, Error> {
        let mut out = Vec::new();
        while let Some(item) = self.next().await {
            out.push(item?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn book(fetched: Arc<Mutex<Vec<Option<String>>>>) -> Pages<'static, u32> {
        Pages::new(Box::new(move |token: Option<String>| {
            fetched.lock().unwrap().push(token.clone());
            Box::pin(async move {
                Ok(match token.as_deref() {
                    None => (vec![1, 2], "b".to_owned()),
                    Some("b") => (vec![3], "c".to_owned()),
                    _ => (vec![4], String::new()),
                })
            })
        }))
    }

    #[tokio::test]
    async fn walks_every_page_in_order() {
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let all = book(fetched.clone()).collect().await.unwrap();
        assert_eq!(all, [1, 2, 3, 4]);
        assert_eq!(
            *fetched.lock().unwrap(),
            [None, Some("b".to_owned()), Some("c".to_owned())]
        );
    }

    #[tokio::test]
    async fn stopping_early_fetches_nothing_more() {
        let fetched = Arc::new(Mutex::new(Vec::new()));
        let mut pages = book(fetched.clone());
        assert_eq!(pages.next().await.unwrap().unwrap(), 1);
        assert_eq!(pages.next().await.unwrap().unwrap(), 2);
        drop(pages);
        assert_eq!(*fetched.lock().unwrap(), [None]);
    }

    #[tokio::test]
    async fn an_error_mid_way_ends_the_walk_after_the_earlier_items() {
        let mut pages: Pages<'static, u32> = Pages::new(Box::new(|token: Option<String>| {
            Box::pin(async move {
                match token {
                    None => Ok((vec![1], "b".to_owned())),
                    Some(_) => Err(Error::Connection {
                        host: "api".into(),
                        reason: "boom".into(),
                    }),
                }
            })
        }));
        assert_eq!(pages.next().await.unwrap().unwrap(), 1);
        assert!(pages.next().await.unwrap().is_err());
        assert!(pages.next().await.is_none());
    }

    #[tokio::test]
    async fn a_repeated_token_ends_the_walk() {
        let calls = Arc::new(Mutex::new(0_u32));
        let c = calls.clone();
        let pages: Pages<'static, u32> = Pages::new(Box::new(move |_| {
            let n = {
                let mut n = c.lock().unwrap();
                *n += 1;
                *n
            };
            Box::pin(async move { Ok((vec![n], "same".to_owned())) })
        }));
        assert_eq!(pages.collect().await.unwrap(), [1, 2]);
    }
}
