//! Validated category and application facts without persistence-specific identity.

use crate::{ApplicationId, CategoryId, UtcMicros};
use core::{fmt, marker::PhantomData};

/// A nonempty display name after Unicode whitespace is trimmed.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ValidatedName<Kind> {
    value: String,
    kind: PhantomData<fn() -> Kind>,
}

impl<Kind> ValidatedName<Kind> {
    /// Validates and trims a category or application display name.
    ///
    /// # Errors
    ///
    /// Returns [`NameError::Empty`] when the input contains no non-whitespace characters.
    pub fn try_new(value: impl AsRef<str>) -> Result<Self, NameError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(NameError::Empty);
        }
        Ok(Self {
            value: value.to_owned(),
            kind: PhantomData,
        })
    }

    /// Returns the normalized display text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Consumes the value and returns the normalized display text.
    #[must_use]
    pub fn into_string(self) -> String {
        self.value
    }
}

impl<Kind> fmt::Debug for ValidatedName<Kind> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ValidatedName")
            .field(&self.value)
            .finish()
    }
}

/// Failure while validating a user-visible category or application name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NameError {
    /// The input was empty after trimming Unicode whitespace.
    Empty,
}

impl fmt::Display for NameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("display name must not be empty after trimming")
    }
}

impl std::error::Error for NameError {}

/// Marker for a category display name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CategoryNameKind {}

/// Validated user-visible category name.
pub type CategoryName = ValidatedName<CategoryNameKind>;

/// Marker for an application display name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ApplicationNameKind {}

/// Validated user-visible application name.
pub type ApplicationName = ValidatedName<ApplicationNameKind>;

/// A category with a stable public ID and validated display value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Category {
    id: CategoryId,
    name: CategoryName,
}

impl Category {
    /// Creates a category from already validated domain values.
    #[must_use]
    pub const fn new(id: CategoryId, name: CategoryName) -> Self {
        Self { id, name }
    }

    /// Returns the stable category ID.
    #[must_use]
    pub const fn id(&self) -> CategoryId {
        self.id
    }

    /// Returns the validated display name.
    #[must_use]
    pub const fn name(&self) -> &CategoryName {
        &self.name
    }
}

/// An application with a current optional category association.
///
/// `None` represents Uncategorized; there is deliberately no synthetic Uncategorized category.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Application {
    id: ApplicationId,
    name: ApplicationName,
    category_id: Option<CategoryId>,
    first_seen: UtcMicros,
    last_seen: UtcMicros,
}

impl Application {
    /// Creates an application with its current optional category association.
    ///
    /// # Errors
    ///
    /// Returns [`ApplicationError::LastSeenBeforeFirstSeen`] when the observation bounds are
    /// reversed.
    pub fn try_new(
        id: ApplicationId,
        name: ApplicationName,
        category_id: Option<CategoryId>,
        first_seen: UtcMicros,
        last_seen: UtcMicros,
    ) -> Result<Self, ApplicationError> {
        if last_seen < first_seen {
            return Err(ApplicationError::LastSeenBeforeFirstSeen {
                first_seen,
                last_seen,
            });
        }
        Ok(Self {
            id,
            name,
            category_id,
            first_seen,
            last_seen,
        })
    }

    /// Returns the stable application ID.
    #[must_use]
    pub const fn id(&self) -> ApplicationId {
        self.id
    }

    /// Returns the validated current display name.
    #[must_use]
    pub const fn name(&self) -> &ApplicationName {
        &self.name
    }

    /// Returns the current category association, or `None` for Uncategorized.
    #[must_use]
    pub const fn category_id(&self) -> Option<CategoryId> {
        self.category_id
    }

    /// Returns the first observed UTC instant.
    #[must_use]
    pub const fn first_seen(&self) -> UtcMicros {
        self.first_seen
    }

    /// Returns the latest observed UTC instant.
    #[must_use]
    pub const fn last_seen(&self) -> UtcMicros {
        self.last_seen
    }
}

/// Failure while constructing an application fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    /// The latest observation was earlier than the first observation.
    LastSeenBeforeFirstSeen {
        /// First observed UTC instant.
        first_seen: UtcMicros,
        /// Requested latest observed UTC instant.
        last_seen: UtcMicros,
    },
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LastSeenBeforeFirstSeen {
                first_seen,
                last_seen,
            } => write!(
                formatter,
                "application last-seen instant {} precedes first-seen instant {}",
                last_seen.get(),
                first_seen.get()
            ),
        }
    }
}

impl std::error::Error for ApplicationError {}

#[cfg(test)]
mod tests {
    use super::{
        Application, ApplicationError, ApplicationName, Category, CategoryName, NameError,
    };
    use crate::{ApplicationId, CategoryId, UtcMicros};

    #[test]
    fn category_and_application_names_trim_but_never_accept_empty_values() {
        let category_name = CategoryName::try_new("  Focused work\t");
        let application_name = ApplicationName::try_new("  Editor  ");
        assert_eq!(
            category_name.map(super::ValidatedName::into_string),
            Ok("Focused work".to_owned())
        );
        assert_eq!(
            application_name.map(super::ValidatedName::into_string),
            Ok("Editor".to_owned())
        );
        assert_eq!(CategoryName::try_new(" \n\t "), Err(NameError::Empty));
        assert_eq!(ApplicationName::try_new(""), Err(NameError::Empty));
    }

    #[test]
    fn applications_keep_optional_current_category_and_ordered_observations() {
        let category_id = CategoryId::from_bytes([1; 16]);
        let category = Category::new(
            category_id,
            CategoryName::try_new("Work").expect("nonempty category"),
        );
        let application = Application::try_new(
            ApplicationId::from_bytes([2; 16]),
            ApplicationName::try_new("Editor").expect("nonempty application"),
            Some(category.id()),
            UtcMicros::new(10),
            UtcMicros::new(10),
        );
        assert_eq!(
            application.map(|value| value.category_id()),
            Ok(Some(category_id))
        );
        assert_eq!(
            Application::try_new(
                ApplicationId::from_bytes([2; 16]),
                ApplicationName::try_new("Editor").expect("nonempty application"),
                None,
                UtcMicros::new(11),
                UtcMicros::new(10),
            ),
            Err(ApplicationError::LastSeenBeforeFirstSeen {
                first_seen: UtcMicros::new(11),
                last_seen: UtcMicros::new(10),
            })
        );
    }
}
