//! Bounded, rebuildable application-icon results for background metadata work.

use std::collections::HashMap;

use openmanic_domain::ApplicationId;

/// Stable content digest for one decoded application icon.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ApplicationIconDigest([u8; 32]);

impl ApplicationIconDigest {
    /// Creates a digest from the authoritative icon-cache content hash.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Key used to share one decoded icon across applications that have the same content.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ApplicationIconKey {
    application_id: ApplicationId,
    digest: ApplicationIconDigest,
}

impl ApplicationIconKey {
    /// Creates a key from the owning application and its content digest.
    #[must_use]
    pub const fn new(application_id: ApplicationId, digest: ApplicationIconDigest) -> Self {
        Self {
            application_id,
            digest,
        }
    }

    /// Returns the application whose metadata request produced this icon.
    #[must_use]
    pub const fn application_id(self) -> ApplicationId {
        self.application_id
    }

    /// Returns the rebuildable content digest.
    #[must_use]
    pub const fn digest(self) -> ApplicationIconDigest {
        self.digest
    }
}

/// A decoded, tightly packed RGBA icon returned by background metadata work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationIcon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl ApplicationIcon {
    /// Validates and stores one tightly packed RGBA image.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationIconError::InvalidDimensions`] or
    /// [`ApplicationIconError::UnexpectedByteLength`] when the decoded image is malformed.
    pub fn try_new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ApplicationIconError> {
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|width| {
                usize::try_from(height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .ok_or(ApplicationIconError::InvalidDimensions)?;
        let expected_byte_len = pixel_count
            .checked_mul(4)
            .ok_or(ApplicationIconError::InvalidDimensions)?;
        if width == 0 || height == 0 {
            return Err(ApplicationIconError::InvalidDimensions);
        }
        if rgba.len() != expected_byte_len {
            return Err(ApplicationIconError::UnexpectedByteLength);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    /// Returns the decoded width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the decoded height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns tightly packed RGBA bytes for renderer upload.
    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    fn byte_len(&self) -> usize {
        self.rgba.len()
    }
}

/// Validation failure for a background-decoded application icon.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationIconError {
    /// The image has a zero or unrepresentable pixel extent.
    InvalidDimensions,
    /// The byte buffer is not a tightly packed RGBA image for the stated dimensions.
    UnexpectedByteLength,
}

/// Fixed upper bounds for the rebuildable decoded-icon cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationIconCacheLimits {
    max_entries: usize,
    max_bytes: usize,
}

impl ApplicationIconCacheLimits {
    /// Validates fixed entry and byte ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationIconCacheLimitError`] when either ceiling is zero.
    pub const fn try_new(
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Self, ApplicationIconCacheLimitError> {
        if max_entries == 0 {
            return Err(ApplicationIconCacheLimitError::ZeroEntries);
        }
        if max_bytes == 0 {
            return Err(ApplicationIconCacheLimitError::ZeroBytes);
        }
        Ok(Self {
            max_entries,
            max_bytes,
        })
    }
}

/// Invalid decoded-icon cache limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationIconCacheLimitError {
    /// A cache must retain at least one entry.
    ZeroEntries,
    /// A cache must have a positive byte budget.
    ZeroBytes,
}

/// Privacy-safe cache counters suitable for local diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationIconCacheDiagnostics {
    entries: usize,
    bytes: usize,
    evictions: u64,
    oversized_rejections: u64,
}

impl ApplicationIconCacheDiagnostics {
    /// Returns the number of currently cached decoded icons.
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }

    /// Returns the current decoded-icon byte total.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Returns the number of normal least-recently-used evictions.
    #[must_use]
    pub const fn evictions(self) -> u64 {
        self.evictions
    }

    /// Returns the number of icons rejected because they exceed the entire byte budget.
    #[must_use]
    pub const fn oversized_rejections(self) -> u64 {
        self.oversized_rejections
    }
}

/// Result of inserting one completed background icon result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationIconCacheInsert {
    /// The result was cached after evicting the stated number of older entries.
    Cached {
        /// Number of older cached icons removed to satisfy the fixed bounds.
        evicted_entries: usize,
    },
    /// The result is valid but exceeds the entire cache byte budget.
    RejectedOversized,
}

/// Immutable result exposed to the UI after a cache lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationIconLookup<'a> {
    /// A decoded icon is ready for renderer upload.
    Ready(&'a ApplicationIcon),
    /// No decoded icon is available; render the deterministic product fallback.
    Fallback,
}

/// One completed background icon lookup, safe to hand to a renderer-side cache.
///
/// The result deliberately contains no executable path, package identity, native handle, or
/// platform error. A missing icon is an ordinary deterministic-fallback outcome rather than an
/// error the UI must interpret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationIconResult {
    /// Background work decoded one icon that may be inserted into the bounded cache.
    Decoded {
        /// Stable key for this decoded icon content.
        key: ApplicationIconKey,
        /// Valid, tightly packed RGBA pixels.
        icon: ApplicationIcon,
    },
    /// No usable icon was available; render the deterministic fallback.
    Fallback {
        /// Application whose icon request completed without a usable decoded image.
        application_id: ApplicationId,
    },
}

impl ApplicationIconResult {
    /// Returns the application whose background metadata request completed.
    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        match self {
            Self::Decoded { key, .. } => key.application_id(),
            Self::Fallback { application_id } => *application_id,
        }
    }

    /// Inserts a decoded icon into the bounded cache, or leaves fallback state unchanged.
    pub fn apply_to(self, cache: &mut ApplicationIconCache) -> Option<ApplicationIconCacheInsert> {
        match self {
            Self::Decoded { key, icon } => Some(cache.insert(key, icon)),
            Self::Fallback { .. } => None,
        }
    }
}

/// A bounded, rebuildable least-recently-used cache populated only by background metadata work.
#[derive(Debug)]
pub struct ApplicationIconCache {
    limits: ApplicationIconCacheLimits,
    entries: HashMap<ApplicationIconKey, CachedIcon>,
    keys_by_application: HashMap<ApplicationId, ApplicationIconKey>,
    bytes: usize,
    next_access: u64,
    evictions: u64,
    oversized_rejections: u64,
}

#[derive(Debug)]
struct CachedIcon {
    icon: ApplicationIcon,
    last_access: u64,
}

impl ApplicationIconCache {
    /// Creates an empty decoded-icon cache with fixed bounds.
    #[must_use]
    pub fn new(limits: ApplicationIconCacheLimits) -> Self {
        Self {
            limits,
            entries: HashMap::new(),
            keys_by_application: HashMap::new(),
            bytes: 0,
            next_access: 0,
            evictions: 0,
            oversized_rejections: 0,
        }
    }

    /// Inserts a completed background result, evicting least-recently-used entries as needed.
    pub fn insert(
        &mut self,
        key: ApplicationIconKey,
        icon: ApplicationIcon,
    ) -> ApplicationIconCacheInsert {
        let byte_len = icon.byte_len();
        if byte_len > self.limits.max_bytes {
            self.oversized_rejections = self.oversized_rejections.saturating_add(1);
            return ApplicationIconCacheInsert::RejectedOversized;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.icon.byte_len());
        }
        self.keys_by_application.insert(key.application_id(), key);
        let mut evicted_entries = 0;
        while self.entries.len() >= self.limits.max_entries
            || self.bytes.saturating_add(byte_len) > self.limits.max_bytes
        {
            let Some(lru_key) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&lru_key) {
                self.bytes = self.bytes.saturating_sub(evicted.icon.byte_len());
                self.keys_by_application.remove(&lru_key.application_id());
                self.evictions = self.evictions.saturating_add(1);
                evicted_entries += 1;
            }
        }
        self.next_access = self.next_access.saturating_add(1);
        self.bytes = self.bytes.saturating_add(byte_len);
        self.entries.insert(
            key,
            CachedIcon {
                icon,
                last_access: self.next_access,
            },
        );
        ApplicationIconCacheInsert::Cached { evicted_entries }
    }

    /// Returns a ready decoded icon or a deterministic fallback marker without OS or filesystem work.
    #[must_use]
    pub fn lookup(&mut self, key: ApplicationIconKey) -> ApplicationIconLookup<'_> {
        self.next_access = self.next_access.saturating_add(1);
        let Some(entry) = self.entries.get_mut(&key) else {
            return ApplicationIconLookup::Fallback;
        };
        entry.last_access = self.next_access;
        ApplicationIconLookup::Ready(&entry.icon)
    }

    /// Returns a ready decoded icon for one application without filesystem or OS work.
    ///
    /// The application-to-key association is rebuilt only as background results arrive. A cache
    /// miss deliberately returns the same deterministic fallback marker as an absent decode.
    #[must_use]
    pub fn lookup_application(
        &mut self,
        application_id: ApplicationId,
    ) -> ApplicationIconLookup<'_> {
        let Some(key) = self.keys_by_application.get(&application_id).copied() else {
            return ApplicationIconLookup::Fallback;
        };
        self.lookup(key)
    }

    /// Returns privacy-safe cache counters without paths, application names, or icon payloads.
    #[must_use]
    pub fn diagnostics(&self) -> ApplicationIconCacheDiagnostics {
        ApplicationIconCacheDiagnostics {
            entries: self.entries.len(),
            bytes: self.bytes,
            evictions: self.evictions,
            oversized_rejections: self.oversized_rejections,
        }
    }
}

#[cfg(test)]
mod tests {
    use openmanic_domain::ApplicationId;

    use super::{
        ApplicationIcon, ApplicationIconCache, ApplicationIconCacheInsert,
        ApplicationIconCacheLimits, ApplicationIconDigest, ApplicationIconKey,
        ApplicationIconLookup, ApplicationIconResult,
    };

    fn key(application_byte: u8, digest_byte: u8) -> ApplicationIconKey {
        ApplicationIconKey::new(
            ApplicationId::from_bytes([application_byte; 16]),
            ApplicationIconDigest::from_bytes([digest_byte; 32]),
        )
    }

    fn icon(byte: u8) -> ApplicationIcon {
        ApplicationIcon::try_new(1, 1, vec![byte; 4]).expect("fixture icon should be valid")
    }

    #[test]
    fn decoded_icons_are_bounded_by_entries_and_bytes_with_lru_eviction() {
        let limits = ApplicationIconCacheLimits::try_new(2, 8).expect("fixture limits are valid");
        let mut cache = ApplicationIconCache::new(limits);
        assert_eq!(
            cache.insert(key(1, 1), icon(1)),
            ApplicationIconCacheInsert::Cached { evicted_entries: 0 }
        );
        assert_eq!(
            cache.insert(key(2, 2), icon(2)),
            ApplicationIconCacheInsert::Cached { evicted_entries: 0 }
        );
        assert!(matches!(
            cache.lookup(key(1, 1)),
            ApplicationIconLookup::Ready(_)
        ));
        assert_eq!(
            cache.insert(key(3, 3), icon(3)),
            ApplicationIconCacheInsert::Cached { evicted_entries: 1 }
        );
        assert!(matches!(
            cache.lookup(key(2, 2)),
            ApplicationIconLookup::Fallback
        ));
        assert!(matches!(
            cache.lookup(key(1, 1)),
            ApplicationIconLookup::Ready(_)
        ));
        assert_eq!(cache.diagnostics().entries(), 2);
        assert_eq!(cache.diagnostics().bytes(), 8);
        assert_eq!(cache.diagnostics().evictions(), 1);
    }

    #[test]
    fn oversize_or_malformed_icons_leave_the_cache_unchanged() {
        let limits = ApplicationIconCacheLimits::try_new(1, 4).expect("fixture limits are valid");
        let mut cache = ApplicationIconCache::new(limits);
        let oversized = ApplicationIcon::try_new(2, 1, vec![0; 8]).expect("fixture icon is valid");
        assert_eq!(
            cache.insert(key(1, 1), oversized),
            ApplicationIconCacheInsert::RejectedOversized
        );
        assert!(matches!(
            cache.lookup(key(1, 1)),
            ApplicationIconLookup::Fallback
        ));
        assert_eq!(cache.diagnostics().oversized_rejections(), 1);
        assert!(ApplicationIcon::try_new(1, 1, vec![0; 3]).is_err());
    }

    #[test]
    fn completed_background_result_exposes_only_safe_cache_data() {
        let key = key(1, 2);
        let mut cache = ApplicationIconCache::new(
            ApplicationIconCacheLimits::try_new(1, 4).expect("fixture limits are valid"),
        );
        let decoded = ApplicationIconResult::Decoded { key, icon: icon(3) };
        assert_eq!(decoded.application_id(), key.application_id());
        assert_eq!(
            decoded.apply_to(&mut cache),
            Some(ApplicationIconCacheInsert::Cached { evicted_entries: 0 })
        );
        let fallback = ApplicationIconResult::Fallback {
            application_id: key.application_id(),
        };
        assert_eq!(fallback.application_id(), key.application_id());
        assert_eq!(fallback.apply_to(&mut cache), None);
    }

    #[test]
    fn application_lookup_follows_decoded_result_and_eviction() {
        let mut cache = ApplicationIconCache::new(
            ApplicationIconCacheLimits::try_new(1, 4).expect("fixture limits are valid"),
        );
        let first = key(1, 1);
        let second = key(2, 2);
        let _ = cache.insert(first, icon(1));
        assert!(matches!(
            cache.lookup_application(first.application_id()),
            ApplicationIconLookup::Ready(_)
        ));
        let _ = cache.insert(second, icon(2));
        assert!(matches!(
            cache.lookup_application(first.application_id()),
            ApplicationIconLookup::Fallback
        ));
        assert!(matches!(
            cache.lookup_application(second.application_id()),
            ApplicationIconLookup::Ready(_)
        ));
    }
}
