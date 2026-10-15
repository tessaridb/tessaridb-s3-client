use super::{
    StoredPart, complete_result, copy_result, error_document, list_buckets, list_objects_v2,
    list_parts, tree, upload_id,
};
use crate::{Error, ErrorCategory};

const NS: &str = "http://s3.amazonaws.com/doc/2006-03-01/";

fn what(result: Result<impl std::fmt::Debug, Error>) -> &'static str {
    match result {
        Err(Error::InvalidResponse { what }) => what,
        other => panic!("expected an invalid response, got {other:?}"),
    }
}

#[test]
fn an_error_document_gives_its_code_message_and_request_id() {
    let body = b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code><Message>The key &amp; more</Message><Resource>/b/k</Resource><RequestId>R1</RequestId></Error>";
    assert_eq!(
        error_document(body),
        Some((
            "NoSuchKey".to_owned(),
            Some("The key & more".to_owned()),
            Some("R1".to_owned())
        ))
    );
    for not_one in [
        &b""[..],
        b"<html>bad gateway</html>",
        b"<Error><Message>x</Message></Error>",
        b"not xml",
    ] {
        assert_eq!(
            error_document(not_one),
            None,
            "{}",
            String::from_utf8_lossy(not_one)
        );
    }
}

#[test]
fn a_dtd_a_foreign_entity_or_deep_nesting_is_refused() {
    let dtd =
        b"<!DOCTYPE x [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><Error><Code>&e;</Code></Error>";
    assert_eq!(what(tree(dtd)), "an XML document declaring a DTD");
    assert_eq!(
        what(tree(b"<Error><Code>&e;</Code></Error>")),
        "an XML document using an undeclared entity"
    );
    let deep = format!("{}{}", "<a>".repeat(9), "</a>".repeat(9));
    assert_eq!(
        what(tree(deep.as_bytes())),
        "an XML document nested too deeply"
    );
    let big = vec![b' '; 8 * 1024 * 1024 + 1];
    assert_eq!(what(tree(&big)), "an XML document over 8 MiB");
    assert_eq!(
        what(tree(b"<a><b></a>")),
        "an XML document that is not well formed"
    );
}

#[test]
fn a_bucket_list_reads_names_and_dates_with_or_without_the_namespace() {
    let body = format!(
        "<ListAllMyBucketsResult xmlns=\"{NS}\"><Owner><ID>o</ID></Owner><Buckets><Bucket><Name>alpha</Name><CreationDate>2026-10-15T08:00:00.000Z</CreationDate></Bucket><Bucket><Name>beta</Name></Bucket></Buckets></ListAllMyBucketsResult>"
    );
    let buckets = list_buckets(body.as_bytes()).map(|all| {
        all.into_iter()
            .map(|bucket| (bucket.name, bucket.created.map(|time| time.to_rfc3339())))
            .collect::<Vec<_>>()
    });
    assert_eq!(
        buckets.ok(),
        Some(vec![
            (
                "alpha".to_owned(),
                Some("2026-10-15T08:00:00+00:00".to_owned())
            ),
            ("beta".to_owned(), None)
        ])
    );
    assert_eq!(
        list_buckets(b"<ListAllMyBucketsResult/>")
            .map(|all| all.len())
            .ok(),
        Some(0)
    );
    assert_eq!(
        what(list_buckets(b"<Other/>")),
        "an answer with an unexpected root element"
    );
}

#[test]
fn a_url_encoded_listing_is_decoded_once() {
    let body = format!(
        "<ListBucketResult xmlns=\"{NS}\"><Name>b</Name><EncodingType>url</EncodingType><IsTruncated>true</IsTruncated><NextContinuationToken>tok=1</NextContinuationToken>\
         <Contents><Key>a+b%2Bc%25%C3%A9/</Key><Size>12</Size><ETag>\"e1\"</ETag><LastModified>2026-10-15T08:00:00.000Z</LastModified></Contents>\
         <CommonPrefixes><Prefix>dir+one/</Prefix></CommonPrefixes></ListBucketResult>"
    );
    let page = list_objects_v2(body.as_bytes());
    let page = page.as_ref().ok();
    assert_eq!(page.map(|p| p.objects[0].key.as_str()), Some("a b+c%é/"));
    assert_eq!(
        page.map(|p| (p.objects[0].size, p.objects[0].etag.as_str())),
        Some((12, "\"e1\""))
    );
    assert_eq!(
        page.map(|p| p.common_prefixes.clone()),
        Some(vec!["dir one/".to_owned()])
    );
    assert_eq!(
        page.and_then(|p| p.next_continuation_token.as_deref()),
        Some("tok=1")
    );
}

#[test]
fn a_plain_listing_keeps_names_as_written_and_the_last_page_has_no_token() {
    let body = b"<ListBucketResult><IsTruncated>false</IsTruncated><Contents><Key>a+b%20</Key><Size>0</Size><ETag>\"e\"</ETag></Contents></ListBucketResult>";
    let page = list_objects_v2(body).ok();
    assert_eq!(
        page.as_ref().map(|p| p.objects[0].key.as_str()),
        Some("a+b%20")
    );
    assert_eq!(page.and_then(|p| p.next_continuation_token), None);
}

#[test]
fn a_listing_that_breaks_its_own_contract_is_refused() {
    let cases: [(&[u8], &str); 4] = [
        (b"<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>", "a truncated listing without a continuation token"),
        (b"<ListBucketResult><EncodingType>url</EncodingType><Contents><Key>%zz</Key><Size>1</Size><ETag>e</ETag></Contents></ListBucketResult>", "a URL-encoded name that does not decode"),
        (b"<ListBucketResult><Contents><Key>k</Key><Size>-1</Size><ETag>e</ETag></Contents></ListBucketResult>", "an object size that is not a number"),
        (b"<ListBucketResult><Contents><Size>1</Size><ETag>e</ETag></Contents></ListBucketResult>", "an answer missing a required element"),
    ];
    for (body, expected) in cases {
        assert_eq!(what(list_objects_v2(body)), expected);
    }
}

#[test]
fn a_copy_answer_gives_its_etag_or_the_error_hidden_in_a_200() {
    assert_eq!(
        copy_result(b"<CopyObjectResult><LastModified>2026-10-15T08:00:00.000Z</LastModified><ETag>\"abc\"</ETag></CopyObjectResult>", 200).ok(),
        Some("\"abc\"".to_owned())
    );
    let hidden = copy_result(
        b"<Error><Code>InternalError</Code><RequestId>R</RequestId></Error>",
        200,
    );
    assert!(
        matches!(&hidden, Err(error) if error.category() == ErrorCategory::Unavailable && error.s3_code() == Some("InternalError"))
    );
}

#[test]
fn multipart_answers_give_the_upload_id_the_parts_and_the_final_etag() {
    assert_eq!(
        upload_id(b"<InitiateMultipartUploadResult><Bucket>b</Bucket><Key>k</Key><UploadId>abc-123</UploadId></InitiateMultipartUploadResult>").ok(),
        Some("abc-123".to_owned())
    );
    assert_eq!(
        what(upload_id(
            b"<InitiateMultipartUploadResult><UploadId></UploadId></InitiateMultipartUploadResult>"
        )),
        "an empty upload ID"
    );
    let page = list_parts(b"<ListPartsResult><IsTruncated>true</IsTruncated><NextPartNumberMarker>2</NextPartNumberMarker><Part><PartNumber>1</PartNumber><ETag>\"e1\"</ETag><Size>8388608</Size></Part><Part><PartNumber>2</PartNumber><ETag>\"e2\"</ETag><Size>5</Size></Part></ListPartsResult>");
    assert_eq!(
        page.ok(),
        Some((
            vec![
                StoredPart {
                    number: 1,
                    etag: "\"e1\"".to_owned(),
                    size: 8_388_608
                },
                StoredPart {
                    number: 2,
                    etag: "\"e2\"".to_owned(),
                    size: 5
                },
            ],
            Some("2".to_owned())
        ))
    );
    assert_eq!(
        what(list_parts(
            b"<ListPartsResult><IsTruncated>true</IsTruncated></ListPartsResult>"
        )),
        "a truncated part list without a marker"
    );
    assert_eq!(what(list_parts(b"<ListPartsResult><Part><PartNumber>10001</PartNumber><ETag>e</ETag><Size>1</Size></Part></ListPartsResult>")), "a part number that is not 1 to 10000");
    assert_eq!(what(list_parts(b"<ListPartsResult><Part><PartNumber>0x1</PartNumber><ETag>e</ETag><Size>1</Size></Part></ListPartsResult>")), "a part number that is not 1 to 10000");
    assert_eq!(
        complete_result(
            b"<CompleteMultipartUploadResult><ETag>\"m-3\"</ETag></CompleteMultipartUploadResult>",
            200
        )
        .ok(),
        Some("\"m-3\"".to_owned())
    );
    let hidden = complete_result(b"<Error><Code>InvalidPart</Code></Error>", 200);
    assert!(
        matches!(&hidden, Err(error) if error.s3_code() == Some("InvalidPart")),
        "{hidden:?}"
    );
}
