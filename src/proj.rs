use geo_types::Point;
use libc::c_int;
use libc::{c_char, c_double};
use num_traits::Float;
use proj_sys::{
    proj_area_create, proj_area_destroy, proj_area_set_bbox, proj_context_create,
    proj_context_destroy, proj_create, proj_create_crs_to_crs, proj_destroy, proj_errno_string,
    proj_normalize_for_visualization, proj_pj_info, proj_trans, proj_trans_array, PJconsts,
    PJ_AREA, PJ_CONTEXT, PJ_COORD, PJ_DIRECTION_PJ_FWD, PJ_DIRECTION_PJ_INV, PJ_LP, PJ_XY,
};
use proj_sys::{proj_errno, proj_errno_reset};
use std::ffi::CStr;
use std::ffi::CString;
use std::str;
use thiserror::Error;

/// Errors originating in PROJ which can occur during projection and conversion
#[derive(Error, Debug)]
pub enum ProjError {
    #[error("The projection failed with the following error: {0}")]
    Projection(String),
    #[error("The conversion failed with the following error: {0}")]
    Conversion(String),
}

/// The bounding box of an area of use
///
/// In the case of an area of use crossing the antimeridian (longitude +/- 180 degrees),
/// `west` must be greater than `east`.
pub struct Area {
    north: f64,
    south: f64,
    east: f64,
    west: f64,
}

impl Area {
    /// Create a new Area
    ///
    /// **Note**: In the case of an area of use crossing the antimeridian (longitude +/- 180 degrees),
    /// `west` must be greater than `east`.
    pub fn new(west: f64, south: f64, east: f64, north: f64) -> Self {
        Area {
            west,
            south,
            east,
            north,
        }
    }
}

/// Easily get a String from the external library
fn _string(raw_ptr: *const c_char) -> String {
    let c_str = unsafe { CStr::from_ptr(raw_ptr) };
    str::from_utf8(c_str.to_bytes()).unwrap().to_string()
}

/// Look up an error message using the error code
fn error_message(code: c_int) -> String {
    let rv = unsafe { proj_errno_string(code) };
    _string(rv)
}

/// Set the bounding box of the area of use
fn area_set_bbox(parea: *mut proj_sys::PJ_AREA, new_area: Option<Area>) {
    // if a bounding box has been passed, modify the proj area object
    if let Some(narea) = new_area {
        unsafe {
            proj_area_set_bbox(parea, narea.west, narea.south, narea.east, narea.north);
        }
    }
}

/// A `PROJ` instance
pub struct Proj {
    c_proj: *mut PJconsts,
    ctx: *mut PJ_CONTEXT,
    area: Option<*mut PJ_AREA>,
}

impl Proj {
    /// Try to instantiate a new `PROJ` instance
    ///
    /// **Note:** for projection operations, `definition` specifies
    /// the **output** projection; input coordinates
    /// are assumed to be geodetic in radians, unless an inverse projection is intended.
    ///
    /// For conversion operations, `definition` defines input, output, and
    /// any intermediate steps that are required. See the `convert` example for more details.
    ///
    /// # Safety
    /// This method contains unsafe code.

    // In contrast to proj v4.x, the type of transformation
    // is signalled by the choice of enum used as input to the PJ_COORD union
    // PJ_LP signals projection of geodetic coordinates, with output being PJ_XY
    // and vice versa, or using PJ_XY for conversion operations
    pub fn new(definition: &str) -> Option<Proj> {
        let c_definition = CString::new(definition.as_bytes()).unwrap();
        let ctx = unsafe { proj_context_create() };
        let new_c_proj = unsafe { proj_create(ctx, c_definition.as_ptr()) };
        // check for unexpected returned object type
        // let return_code: i32 = unsafe { proj_get_type(new_c_proj) };
        if new_c_proj.is_null() {
            None
        } else {
            Some(Proj {
                c_proj: new_c_proj,
                ctx,
                area: None,
            })
        }
    }

    /// Create a transformation object that is a pipeline between two known coordinate reference systems.
    /// `from` and `to` can be:
    ///
    /// - an `"AUTHORITY:CODE"`, like `"EPSG:25832"`.
    /// - a PROJ string, like `"+proj=longlat +datum=WGS84"`. When using that syntax, the unit is expected to be degrees.
    /// - the name of a CRS as found in the PROJ database, e.g `"WGS84"`, `"NAD27"`, etc.
    /// - more generally, any string accepted by [`new()`](struct.Proj.html#method.new)
    ///
    /// If you wish to alter the particular area of use, you may do so using [`area_set_bbox()`](struct.Proj.html#method.area_set_bbox)
    /// ## A Note on Coordinate Order
    /// The required input **and** output coordinate order is **normalised** to `Longitude, Latitude` / `Easting, Northing`.
    ///
    /// This overrides the expected order of the specified input and / or output CRS if necessary.
    /// See the [PROJ API](https://proj.org/development/reference/functions.html#c.proj_normalize_for_visualization)
    ///
    /// For example: per its definition, EPSG:4326 has an axis order of Latitude, Longitude. Without
    /// normalisation, crate users would have to
    /// [remember](https://proj.org/development/reference/functions.html#c.proj_create_crs_to_crs)
    /// to reverse the coordinates of `Point` or `Coordinate` structs in order for a conversion operation to
    /// return correct results.
    ///
    ///```rust
    /// # use assert_approx_eq::assert_approx_eq;
    /// extern crate proj;
    /// use proj::Proj;
    ///
    /// extern crate geo_types;
    /// use geo_types::Point;
    ///
    /// let from = "EPSG:2230";
    /// let to = "EPSG:26946";
    /// let nad_ft_to_m = Proj::new_known_crs(&from, &to, None).unwrap();
    /// let result = nad_ft_to_m
    ///     .convert(Point::new(4760096.421921f64, 3744293.729449f64))
    ///     .unwrap();
    /// assert_approx_eq!(result.x(), 1450880.29f64, 1.0e-2);
    /// assert_approx_eq!(result.y(), 1141263.01f64, 1.0e-2);
    /// ```
    ///
    /// # Safety
    /// This method contains unsafe code.
    pub fn new_known_crs(from: &str, to: &str, area: Option<Area>) -> Option<Proj> {
        let from_c = CString::new(from.as_bytes()).unwrap();
        let to_c = CString::new(to.as_bytes()).unwrap();
        let ctx = unsafe { proj_context_create() };
        let proj_area = unsafe { proj_area_create() };
        area_set_bbox(proj_area, area);
        let new_c_proj =
            unsafe { proj_create_crs_to_crs(ctx, from_c.as_ptr(), to_c.as_ptr(), proj_area) };
        if new_c_proj.is_null() {
            None
        } else {
            // Normalise input and output order to Lon, Lat / Easting Northing by inserting
            // An axis swap operation if necessary
            let normalised = unsafe {
                let normalised = proj_normalize_for_visualization(ctx, new_c_proj);
                // deallocate stale PJ pointer
                proj_destroy(new_c_proj);
                normalised
            };
            Some(Proj {
                c_proj: normalised,
                ctx,
                area: Some(proj_area),
            })
        }
    }

    /// Set the bounding box of the area of use
    ///
    /// This bounding box will be used to specify the area of use
    /// for the choice of relevant coordinate operations.
    /// In the case of an area of use crossing the antimeridian (longitude +/- 180 degrees),
    /// `west` **must** be greater than `east`.
    ///
    /// # Safety
    /// This method contains unsafe code.
    // calling this on a non-CRS-to-CRS instance of Proj will be harmless, because self.area will be None
    pub fn area_set_bbox(&mut self, new_bbox: Area) {
        if let Some(new_area) = self.area {
            unsafe {
                proj_area_set_bbox(
                    new_area,
                    new_bbox.west,
                    new_bbox.south,
                    new_bbox.east,
                    new_bbox.north,
                );
            }
        }
    }

    /// Get the current definition from `PROJ`
    ///
    /// # Safety
    /// This method contains unsafe code.
    pub fn def(&self) -> String {
        let rv = unsafe { proj_pj_info(self.c_proj) };
        _string(rv.definition)
    }
    /// Project geodetic coordinates (in radians) into the projection specified by `definition`
    ///
    /// **Note:** specifying `inverse` as `true` carries out an inverse projection *to* geodetic coordinates
    /// (in radians) from the projection specified by `definition`.
    ///
    /// # Safety
    /// This method contains unsafe code.
    pub fn project<T, U>(&self, point: T, inverse: bool) -> Result<Point<U>, ProjError>
    where
        T: Into<Point<U>>,
        U: Float,
    {
        let inv = if inverse {
            PJ_DIRECTION_PJ_INV
        } else {
            PJ_DIRECTION_PJ_FWD
        };
        let _point: Point<U> = point.into();
        let c_x: c_double = _point.x().to_f64().unwrap();
        let c_y: c_double = _point.y().to_f64().unwrap();
        let new_x;
        let new_y;
        let err;
        // Input coords are defined in terms of lambda & phi, using the PJ_LP struct.
        // This signals that we wish to project geodetic coordinates.
        // For conversion (i.e. between projected coordinates) you should use
        // PJ_XY {x: , y: }
        let coords = PJ_LP { lam: c_x, phi: c_y };
        unsafe {
            proj_errno_reset(self.c_proj);
            // PJ_DIRECTION_* determines a forward or inverse projection
            let trans = proj_trans(self.c_proj, inv, PJ_COORD { lp: coords });
            // output of coordinates uses the PJ_XY struct
            new_x = trans.xy.x;
            new_y = trans.xy.y;
            err = proj_errno(self.c_proj);
        }
        if err == 0 {
            Ok(Point::new(U::from(new_x).unwrap(), U::from(new_y).unwrap()))
        } else {
            Err(ProjError::Projection(error_message(err)))
        }
    }

    /// Convert projected coordinates between coordinate reference systems.
    ///
    /// Input and output CRS may be specified in two ways:
    /// 1. Using the PROJ `pipeline` operator. This method makes use of the [`pipeline`](http://proj4.org/operations/pipeline.html)
    /// functionality available since `PROJ` 5.
    /// This has the advantage of being able to chain an arbitrary combination of projection, conversion,
    /// and transformation steps, allowing for extremely complex operations ([`new`](#method.new))
    /// 2. Using EPSG codes or `PROJ` strings to define input and output CRS ([`new_known_crs`](#method.new_known_crs))
    ///
    /// ## A Note on Coordinate Order
    /// Depending on the method used to instantiate the `Proj` object, coordinate input and output order may vary:
    /// - If you have used [`new`](#method.new), it is assumed that you've specified the order using the input string,
    /// or that you are aware of the required input order and expected output order.
    /// - If you have used [`new_known_crs`](#method.new_known_crs), input and output order are **normalised**
    /// to Longitude, Latitude / Easting, Northing.
    ///
    /// The following example converts from NAD83 US Survey Feet (EPSG 2230) to NAD83 Metres (EPSG 26946)
    /// Note the steps:
    ///
    /// - define the operation as a `pipeline` operation
    /// - define `step` 1 as an `inv`erse transform, yielding geodetic coordinates
    /// - define `step` 2 as a forward transform to projected coordinates, yielding metres.
    ///
    /// ```rust
    /// # use assert_approx_eq::assert_approx_eq;
    /// extern crate proj;
    /// use proj::Proj;
    ///
    /// extern crate geo_types;
    /// use geo_types::Point;
    ///
    /// let nad_ft_to_m = Proj::new(
    ///     "
    ///     +proj=pipeline
    ///     +step +inv +proj=lcc +lat_1=33.88333333333333
    ///     +lat_2=32.78333333333333 +lat_0=32.16666666666666
    ///     +lon_0=-116.25 +x_0=2000000.0001016 +y_0=500000.0001016001 +ellps=GRS80
    ///     +towgs84=0,0,0,0,0,0,0 +units=us-ft +no_defs
    ///     +step +proj=lcc +lat_1=33.88333333333333 +lat_2=32.78333333333333 +lat_0=32.16666666666666
    ///     +lon_0=-116.25 +x_0=2000000 +y_0=500000
    ///     +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs
    /// ",
    /// )
    /// .unwrap();
    /// let result = nad_ft_to_m
    ///     .convert(Point::new(4760096.421921f64, 3744293.729449f64))
    ///     .unwrap();
    /// assert_approx_eq!(result.x(), 1450880.29f64, 1.0e-2);
    /// assert_approx_eq!(result.y(), 1141263.01f64, 1.0e-2);
    /// ```
    ///
    /// # Safety
    /// This method contains unsafe code.
    pub fn convert<T, U>(&self, point: T) -> Result<Point<U>, ProjError>
    where
        T: Into<Point<U>>,
        U: Float,
    {
        let _point: Point<U> = point.into();
        let c_x: c_double = _point.x().to_f64().unwrap();
        let c_y: c_double = _point.y().to_f64().unwrap();
        let new_x;
        let new_y;
        let err;
        let coords = PJ_XY { x: c_x, y: c_y };
        unsafe {
            proj_errno_reset(self.c_proj);
            let trans = proj_trans(self.c_proj, PJ_DIRECTION_PJ_FWD, PJ_COORD { xy: coords });
            new_x = trans.xy.x;
            new_y = trans.xy.y;
            err = proj_errno(self.c_proj);
        }
        if err == 0 {
            Ok(Point::new(U::from(new_x).unwrap(), U::from(new_y).unwrap()))
        } else {
            Err(ProjError::Conversion(error_message(err)))
        }
    }

    /// Convert a mutable slice (or anything that can deref into a mutable slice) of `Point`s
    ///
    /// The following example converts from NAD83 US Survey Feet (EPSG 2230) to NAD83 Metres (EPSG 26946)
    ///
    /// ## A Note on Coordinate Order
    /// Depending on the method used to instantiate the `Proj` object, coordinate input and output order may vary:
    /// - If you have used [`new`](#method.new), it is assumed that you've specified the order using the input string,
    /// or that you are aware of the required input order and expected output order.
    /// - If you have used [`new_known_crs`](#method.new_known_crs), input and output order are **normalised**
    /// to Longitude, Latitude / Easting, Northing.
    ///
    /// ```rust
    /// use proj::Proj;
    /// extern crate geo_types;
    /// use geo_types::Point;
    /// # use assert_approx_eq::assert_approx_eq;
    /// let from = "EPSG:2230";
    /// let to = "EPSG:26946";
    /// let ft_to_m = Proj::new_known_crs(&from, &to, None).unwrap();
    /// let mut v = vec![
    ///     Point::new(4760096.421921, 3744293.729449),
    ///     Point::new(4760197.421921, 3744394.729449),
    /// ];
    /// ft_to_m.convert_array(&mut v);
    /// assert_approx_eq!(v[0].x(), 1450880.2910605003f64);
    /// assert_approx_eq!(v[1].y(), 1141293.7960220212f64);
    /// ```
    ///
    /// # Safety
    /// This method contains unsafe code.
    // TODO: there may be a way of avoiding some allocations, but transmute won't work because
    // PJ_COORD and Point<T> are different sizes
    pub fn convert_array<'a, T>(
        &self,
        points: &'a mut [Point<T>],
    ) -> Result<&'a mut [Point<T>], ProjError>
    where
        T: Float,
    {
        let err;
        let trans;
        // we need PJ_COORD to convert
        let mut pj = points
            .iter()
            .map(|point| {
                let c_x: c_double = point.x().to_f64().unwrap();
                let c_y: c_double = point.y().to_f64().unwrap();
                PJ_COORD {
                    xy: PJ_XY { x: c_x, y: c_y },
                }
            })
            .collect::<Vec<_>>();
        pj.shrink_to_fit();
        unsafe {
            proj_errno_reset(self.c_proj);
            trans = proj_trans_array(self.c_proj, PJ_DIRECTION_PJ_FWD, pj.len(), pj.as_mut_ptr());
            err = proj_errno(self.c_proj);
        }
        if err == 0 && trans == 0 {
            unsafe {
                // re-fill original slice with Points
                // feels a bit clunky, but we're guaranteed that pj and points have the same length
                pj.iter().enumerate().for_each(|(i, coord)| {
                    points[i] =
                        Point::new(T::from(coord.xy.x).unwrap(), T::from(coord.xy.y).unwrap())
                });
                Ok(points)
            }
        } else {
            Err(ProjError::Projection(error_message(err)))
        }
    }

    /// Project an array of geodetic coordinates (in radians) into the projection specified by `definition`
    ///
    /// **Note:** specifying `inverse` as `true` carries out an inverse projection *to* geodetic coordinates
    /// (in radians) from the projection specified by `definition`.
    ///
    /// ```rust
    /// use proj::Proj;
    /// extern crate geo_types;
    /// use geo_types::Point;
    /// # use assert_approx_eq::assert_approx_eq;
    /// let stereo70 = Proj::new(
    ///     "+proj=sterea +lat_0=46 +lon_0=25 +k=0.99975 +x_0=500000 +y_0=500000
    ///     +ellps=krass +towgs84=33.4,-146.6,-76.3,-0.359,-0.053,0.844,-0.84 +units=m +no_defs",
    /// )
    /// .unwrap();
    /// // Geodetic -> Pulkovo 1942(58) / Stereo70 (EPSG 3844)
    /// let mut v = vec![Point::new(0.436332, 0.802851)];
    /// let t = stereo70.project_array(&mut v, false).unwrap();
    /// assert_approx_eq!(v[0].x(), 500119.7035366755f64);
    /// assert_approx_eq!(v[0].y(), 500027.77901023754f64);
    /// ```
    ///
    /// # Safety
    /// This method contains unsafe code.
    // TODO: there may be a way of avoiding some allocations, but transmute won't work because
    // PJ_COORD and Point<T> are different sizes
    pub fn project_array<'a, T>(
        &self,
        points: &'a mut [Point<T>],
        inverse: bool,
    ) -> Result<&'a mut [Point<T>], ProjError>
    where
        T: Float,
    {
        let err;
        let trans;
        let inv = if inverse {
            PJ_DIRECTION_PJ_INV
        } else {
            PJ_DIRECTION_PJ_FWD
        };
        // we need PJ_COORD to convert
        let mut pj = points
            .iter()
            .map(|point| {
                let c_x: c_double = point.x().to_f64().unwrap();
                let c_y: c_double = point.y().to_f64().unwrap();
                PJ_COORD {
                    xy: PJ_XY { x: c_x, y: c_y },
                }
            })
            .collect::<Vec<_>>();
        pj.shrink_to_fit();
        unsafe {
            proj_errno_reset(self.c_proj);
            trans = proj_trans_array(self.c_proj, inv, pj.len(), pj.as_mut_ptr());
            err = proj_errno(self.c_proj);
        }
        if err == 0 && trans == 0 {
            unsafe {
                // re-fill original slice with Points
                // feels a bit clunky, but we're guaranteed that pj and points have the same length
                pj.iter().enumerate().for_each(|(i, coord)| {
                    points[i] =
                        Point::new(T::from(coord.xy.x).unwrap(), T::from(coord.xy.y).unwrap())
                });
                Ok(points)
            }
        } else {
            Err(ProjError::Projection(error_message(err)))
        }
    }
}

impl Drop for Proj {
    fn drop(&mut self) {
        unsafe {
            proj_destroy(self.c_proj);
            proj_context_destroy(self.ctx);
            if let Some(area) = self.area {
                proj_area_destroy(area)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::Proj;
    use geo_types::Point;

    fn assert_almost_eq(a: f64, b: f64) {
        let f: f64 = a / b;
        assert!(f < 1.00001);
        assert!(f > 0.99999);
    }
    #[test]
    fn test_definition() {
        let wgs84 = "+proj=longlat +datum=WGS84 +no_defs";
        let proj = Proj::new(wgs84).unwrap();
        assert_eq!(
            proj.def(),
            "proj=longlat datum=WGS84 no_defs ellps=WGS84 towgs84=0,0,0"
        );
    }
    #[test]
    fn test_from_crs() {
        let from = "EPSG:2230";
        let to = "EPSG:26946";
        let proj = Proj::new_known_crs(&from, &to, None).unwrap();
        let t = proj
            .convert(Point::new(4760096.421921, 3744293.729449))
            .unwrap();
        assert_almost_eq(t.x(), 1450880.29);
        assert_almost_eq(t.y(), 1141263.01);
    }
    #[test]
    // Carry out a projection from geodetic coordinates
    fn test_projection() {
        let stereo70 = Proj::new(
            "+proj=sterea +lat_0=46 +lon_0=25 +k=0.99975 +x_0=500000 +y_0=500000
            +ellps=krass +towgs84=33.4,-146.6,-76.3,-0.359,-0.053,0.844,-0.84 +units=m +no_defs",
        )
        .unwrap();
        // Geodetic -> Pulkovo 1942(58) / Stereo70 (EPSG 3844)
        let t = stereo70
            .project(Point::new(0.436332, 0.802851), false)
            .unwrap();
        assert_almost_eq(t.x(), 500119.7035366755);
        assert_almost_eq(t.y(), 500027.77901023754);
    }
    #[test]
    // Carry out an inverse projection to geodetic coordinates
    fn test_inverse_projection() {
        let stereo70 = Proj::new(
            "+proj=sterea +lat_0=46 +lon_0=25 +k=0.99975 +x_0=500000 +y_0=500000
            +ellps=krass +towgs84=33.4,-146.6,-76.3,-0.359,-0.053,0.844,-0.84 +units=m +no_defs",
        )
        .unwrap();
        // Pulkovo 1942(58) / Stereo70 (EPSG 3844) -> Geodetic
        let t = stereo70
            .project(Point::new(500119.70352012233, 500027.77896348457), true)
            .unwrap();
        assert_almost_eq(t.x(), 0.436332);
        assert_almost_eq(t.y(), 0.802851);
    }
    #[test]
    // Carry out an inverse projection to geodetic coordinates
    fn test_london_inverse() {
        let osgb36 = Proj::new(
            "
            +proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000 +ellps=airy
            +towgs84=446.448,-125.157,542.06,0.15,0.247,0.842,-20.489 +units=m +no_defs
            ",
        )
        .unwrap();
        // OSGB36 (EPSG 27700) -> Geodetic
        let t = osgb36
            .project(Point::new(548295.39, 182498.46), true)
            .unwrap();
        assert_almost_eq(t.x(), 0.0023755864848281206);
        assert_almost_eq(t.y(), 0.8992274896304518);
    }
    #[test]
    // Carry out a conversion from NAD83 feet (EPSG 2230) to NAD83 metres (EPSG 26946)
    fn test_conversion() {
        let nad83_m = Proj::new("
            +proj=pipeline
            +step +inv +proj=lcc +lat_1=33.88333333333333
            +lat_2=32.78333333333333 +lat_0=32.16666666666666
            +lon_0=-116.25 +x_0=2000000.0001016 +y_0=500000.0001016001 +ellps=GRS80
            +towgs84=0,0,0,0,0,0,0 +units=us-ft +no_defs
            +step +proj=lcc +lat_1=33.88333333333333 +lat_2=32.78333333333333 +lat_0=32.16666666666666
            +lon_0=-116.25 +x_0=2000000 +y_0=500000
            +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs
        ").unwrap();
        // Presidio, San Francisco
        let t = nad83_m
            .convert(Point::new(4760096.421921, 3744293.729449))
            .unwrap();
        assert_almost_eq(t.x(), 1450880.29);
        assert_almost_eq(t.y(), 1141263.01);
    }
    #[test]
    // Test that instantiation fails wth bad proj string input
    fn test_init_error() {
        assert!(Proj::new("🦀").is_none());
    }
    #[test]
    fn test_conversion_error() {
        // because step 1 isn't an inverse conversion, it's expecting lon lat input
        let nad83_m = Proj::new(
            "+proj=geos +lon_0=0.00 +lat_0=0.00 +a=6378169.00 +b=6356583.80 +h=35785831.0",
        )
        .unwrap();
        let err = nad83_m
            .convert(Point::new(4760096.421921, 3744293.729449))
            .unwrap_err();
        assert_eq!(
            "The conversion failed with the following error: latitude or longitude exceeded limits",
            err.to_string()
        );
    }

    #[test]
    fn test_error_recovery() {
        let nad83_m = Proj::new(
            "+proj=geos +lon_0=0.00 +lat_0=0.00 +a=6378169.00 +b=6356583.80 +h=35785831.0",
        )
        .unwrap();

        // we expect this first conversion to fail (copied from above test case)
        assert!(nad83_m
            .convert(Point::new(4760096.421921, 3744293.729449))
            .is_err());

        // but a subsequent valid conversion should still be successful
        assert!(nad83_m.convert(Point::new(0.0, 0.0)).is_ok());

        // also test with project() function
        assert!(nad83_m
            .project(Point::new(99999.0, 99999.0), false)
            .is_err());
        assert!(nad83_m.project(Point::new(0.0, 0.0), false).is_ok());
    }

    #[test]
    fn test_array_convert() {
        let from = "EPSG:2230";
        let to = "EPSG:26946";
        let ft_to_m = Proj::new_known_crs(&from, &to, None).unwrap();
        let mut v = vec![
            Point::new(4760096.421921, 3744293.729449),
            Point::new(4760197.421921, 3744394.729449),
        ];
        ft_to_m.convert_array(&mut v).unwrap();
        assert_almost_eq(v[0].x(), 1450880.2910605003f64);
        assert_almost_eq(v[1].y(), 1141293.7960220212f64);
    }

    #[test]
    // Ensure that input and output order are normalised to Lon, Lat / Easting Northing
    // Without normalisation this test would fail, as EPSG:4326 expects Lat, Lon input order.
    fn test_input_order() {
        let from = "EPSG:4326";
        let to = "EPSG:2230";
        let to_feet = Proj::new_known_crs(&from, &to, None).unwrap();
        // 👽
        let usa_m = Point::new(-115.797615, 37.2647978);
        let usa_ft = to_feet.convert(usa_m).unwrap();
        assert_eq!(6693625.67217475, usa_ft.x());
        assert_eq!(3497301.5918027186, usa_ft.y());
    }
}
