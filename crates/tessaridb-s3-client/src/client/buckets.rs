//! Bucket operations.

use reqwest::Method;

use super::{Client, DEFAULT_REGION};
use crate::transport::Request;
use crate::{BucketInfo, BucketName};
use crate::{Error, Result, xml};

impl Client {
    /// Every bucket the credentials own.
    ///
    /// # Errors
    /// The server's refusal, or a transport failure after retries.
    pub async fn list_buckets(&self) -> Result<Vec<BucketInfo>> {
        let response = self.call(&Request::new(Method::GET, None, None)).await?;
        xml::list_buckets(&response.body)
    }

    /// Creates `bucket` in the client's region. A retry that finds the bucket already created by the first attempt
    /// counts as success.
    ///
    /// # Errors
    /// `Conflict` when the bucket exists, or another refusal.
    pub async fn create_bucket(&self, bucket: &BucketName) -> Result<()> {
        let mut request = Request::new(Method::PUT, Some(bucket), None);
        if self.region != DEFAULT_REGION {
            request.body = format!(
                "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>",
                self.region
            )
            .into();
        }
        let request = &request;
        self.retry
            .run(|attempt| async move {
                match self.transport.send(request).await {
                    Err(error)
                        if attempt > 1 && error.s3_code() == Some("BucketAlreadyOwnedByYou") =>
                    {
                        Ok(())
                    }
                    other => other.map(drop),
                }
            })
            .await
    }

    /// Deletes `bucket`, which must be empty.
    ///
    /// # Errors
    /// `Conflict` (`BucketNotEmpty`), `NotFound`, or another refusal.
    pub async fn delete_bucket(&self, bucket: &BucketName) -> Result<()> {
        self.call(&Request::new(Method::DELETE, Some(bucket), None))
            .await
            .map(drop)
    }

    /// Whether `bucket` exists and the credentials may use it.
    ///
    /// # Errors
    /// Any answer other than found or not found — a forbidden bucket is an error, not `false`.
    pub async fn bucket_exists(&self, bucket: &BucketName) -> Result<bool> {
        match self
            .call(&Request::new(Method::HEAD, Some(bucket), None))
            .await
        {
            Ok(_) => Ok(true),
            Err(Error::Service { status: 404, .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }
}
