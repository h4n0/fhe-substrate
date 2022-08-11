//! Create parameters for the BFV encryption scheme

use derive_builder::Builder;
use math::{
	rns::{RnsContext, ScalingFactor},
	rq::{scaler::Scaler, traits::TryConvertFrom, Context, Poly, Representation},
	zq::{nfl::generate_prime, ntt::NttOperator, Modulus},
};
use ndarray::ArrayView1;
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use std::rc::Rc;

/// Parameters for the BFV encryption scheme.
#[derive(Debug, Builder, PartialEq, Eq)]
#[builder(build_fn(private, name = "fallible_build"))]
pub struct BfvParameters {
	/// Number of coefficients in a polynomial.
	pub(crate) polynomial_degree: usize,

	/// Modulus of the plaintext.
	plaintext_modulus: u64,

	/// Vector of coprime moduli q_i for the ciphertext.
	/// One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified.
	pub(crate) ciphertext_moduli: Vec<u64>,

	/// Vector of the sized of the coprime moduli q_i for the ciphertext.
	/// One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified.
	ciphertext_moduli_sizes: Vec<usize>,

	/// Error variance
	pub(crate) variance: usize,

	/// Context for the underlying polynomials
	#[builder(setter(skip))]
	pub(crate) ctx: Rc<Context>,

	/// Ntt operator for the SIMD plaintext, if possible.
	#[builder(setter(skip))]
	pub(crate) op: Option<Rc<NttOperator>>,

	/// Scaling polynomial for the plaintext
	#[builder(setter(skip))]
	pub(crate) delta: Poly,

	/// Q modulo the plaintext modulus
	#[builder(setter(skip))]
	pub(crate) q_mod_t: u64,

	/// Down scaler for the plaintext
	#[builder(setter(skip))]
	pub(crate) scaler: Scaler,

	/// Plaintext Modulus
	// #[builder(setter(skip))] // TODO: How can we handle this?
	pub(crate) plaintext: Modulus,

	// Parameters for the multiplications
	#[builder(setter(skip))]
	pub(crate) mul_1_params: MultiplicationParameters, // type 1
	#[builder(setter(skip))]
	pub(crate) mul_2_params: MultiplicationParameters, // type 2

	#[builder(setter(skip))]
	pub(crate) matrix_reps_index_map: Vec<usize>,
}

impl BfvParameters {
	/// Returns the underlying polynomial degree
	pub fn degree(&self) -> usize {
		self.polynomial_degree
	}

	/// Returns a reference to the ciphertext moduli
	pub fn moduli(&self) -> &[u64] {
		&self.ciphertext_moduli
	}

	/// Returns a reference to the ciphertext moduli
	pub fn moduli_sizes(&self) -> &[usize] {
		&self.ciphertext_moduli_sizes
	}

	#[cfg(test)]
	pub fn default(num_moduli: usize) -> Self {
		BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(1153)
			.ciphertext_moduli_sizes(vec![62; num_moduli])
			.build()
			.unwrap()
	}
}

impl BfvParametersBuilder {
	/// Build a new `BfvParameters`.
	pub fn build(&self) -> Result<BfvParameters, BfvParametersBuilderError> {
		// Check the polynomial degree
		if self.polynomial_degree.is_none() {
			return Err(BfvParametersBuilderError::UninitializedField(
				"polynomial_degree",
			));
		}
		let polynomial_degree = self.polynomial_degree.unwrap();
		if polynomial_degree < 8 || !polynomial_degree.is_power_of_two() {
			return Err(BfvParametersBuilderError::ValidationError(
				"`polynomial_degree` must be a power of two larger or equal to 8".to_string(),
			));
		}

		// Check the plaintext modulus
		if self.plaintext_modulus.is_none() {
			return Err(BfvParametersBuilderError::UninitializedField(
				"plaintext_modulus",
			));
		}
		let plaintext_modulus = Modulus::new(self.plaintext_modulus.unwrap())?;

		// Check the ciphertext moduli
		if self.ciphertext_moduli.is_none() && self.ciphertext_moduli_sizes.is_none() {
			return Err(
				BfvParametersBuilderError::ValidationError("One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified"
					.to_string())
			);
		}

		// Construct the vector of ciphertext moduli.
		let mut ciphertext_moduli = self.ciphertext_moduli.clone().unwrap_or_default();
		let mut ciphertext_moduli_sizes = self.ciphertext_moduli_sizes.clone().unwrap_or_default();

		if (ciphertext_moduli.is_empty() && ciphertext_moduli_sizes.is_empty())
			|| (!ciphertext_moduli.is_empty() && !ciphertext_moduli_sizes.is_empty())
		{
			return Err(
				BfvParametersBuilderError::ValidationError("One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified"
					.to_string())
			);
		} else if ciphertext_moduli_sizes.is_empty() {
			for modulus in &ciphertext_moduli {
				ciphertext_moduli_sizes.push(64 - modulus.leading_zeros() as usize)
			}
		} else {
			ciphertext_moduli = Self::generate_moduli(&ciphertext_moduli_sizes, polynomial_degree)?
		}
		assert_eq!(ciphertext_moduli.len(), ciphertext_moduli_sizes.len());

		let variance = self.variance.unwrap_or(1);
		if !(1..=16).contains(&variance) {
			return Err(BfvParametersBuilderError::ValidationError(
				"The variance should be an integer between 1 and 16".to_string(),
			));
		}

		let op = NttOperator::new(&plaintext_modulus, polynomial_degree);

		// Compute the scaling factors for the plaintext
		let rns = RnsContext::new(&ciphertext_moduli)?;

		let ctx = Rc::new(Context::new(&ciphertext_moduli, polynomial_degree)?);
		let plaintext_ctx = Rc::new(Context::new(&ciphertext_moduli[..1], polynomial_degree)?);
		let scaler = Scaler::new(
			&ctx,
			&plaintext_ctx,
			ScalingFactor::new(&BigUint::from(plaintext_modulus.modulus()), rns.modulus()),
		)?;

		// Compute the NttShoup representation of delta = -1/t mod Q
		let mut delta_rests = vec![];
		for m in &ciphertext_moduli {
			let q = Modulus::new(*m).unwrap();
			delta_rests.push(q.inv(q.neg(plaintext_modulus.modulus())).unwrap())
		}
		let delta = rns.lift(&ArrayView1::from(&delta_rests)); // -1/t mod Q
		let mut delta_poly = Poly::try_convert_from(&[delta], &ctx, Representation::PowerBasis)?;
		delta_poly.change_representation(Representation::NttShoup);

		// Compute Q mod t
		let q_mod_t = (rns.modulus() % plaintext_modulus.modulus())
			.to_u64()
			.unwrap();

		// Create n+1 moduli of 62 bits for multiplication.
		let mut extended_basis = Vec::with_capacity(ciphertext_moduli.len() + 1);
		let mut upper_bound = 1 << 62;
		while extended_basis.len() != ciphertext_moduli.len() + 1 {
			upper_bound = generate_prime(62, 2 * polynomial_degree as u64, upper_bound).unwrap();
			if !extended_basis.contains(&upper_bound) && !ciphertext_moduli.contains(&upper_bound) {
				extended_basis.push(upper_bound)
			}
		}

		// For the first multiplication, we want to extend to a context that is ~60 bits larger.
		let modulus_size = ciphertext_moduli_sizes.iter().sum::<usize>();
		let n_moduli = ((modulus_size + 60) + 61) / 62;
		let mut mul_1_moduli = vec![];
		mul_1_moduli.append(&mut ciphertext_moduli.clone());
		mul_1_moduli.append(&mut extended_basis[..n_moduli].to_vec());
		let mul_1_ctx = Rc::new(Context::new(&mul_1_moduli, polynomial_degree)?);
		let mul_1_params = MultiplicationParameters::new(
			&ctx,
			&mul_1_ctx,
			ScalingFactor::one(),
			ScalingFactor::one(),
			ScalingFactor::new(&BigUint::from(plaintext_modulus.modulus()), ctx.modulus()),
		)?;

		// For the second multiplication, we use two moduli of roughly the same size
		let n_moduli = ciphertext_moduli.len();
		let mut mul_2_moduli = vec![];
		mul_2_moduli.append(&mut ciphertext_moduli.clone());
		mul_2_moduli.append(&mut extended_basis[..n_moduli].to_vec());
		let rns_2 = RnsContext::new(&extended_basis[..n_moduli])?;
		let mul_2_ctx = Rc::new(Context::new(&mul_2_moduli, polynomial_degree)?);
		let mul_2_params = MultiplicationParameters::new(
			&ctx,
			&mul_2_ctx,
			ScalingFactor::one(),
			ScalingFactor::new(rns_2.modulus(), ctx.modulus()),
			ScalingFactor::new(&BigUint::from(plaintext_modulus.modulus()), rns_2.modulus()),
		)?;

		// We use the same code as SEAL
		// https://github.com/microsoft/SEAL/blob/82b07db635132e297282649e2ab5908999089ad2/native/src/seal/batchencoder.cpp
		let row_size = polynomial_degree >> 1;
		let m = polynomial_degree << 1;
		let gen = 3;
		let mut pos = 1;
		let mut matrix_reps_index_map = vec![0usize; polynomial_degree];
		for i in 0..row_size {
			let index1 = (pos - 1) >> 1;
			let index2 = (m - pos - 1) >> 1;
			matrix_reps_index_map[i] =
				index1.reverse_bits() >> (polynomial_degree.leading_zeros() + 1);
			matrix_reps_index_map[row_size | i] =
				index2.reverse_bits() >> (polynomial_degree.leading_zeros() + 1);
			pos *= gen;
			pos &= m - 1;
		}

		Ok(BfvParameters {
			polynomial_degree,
			plaintext_modulus: plaintext_modulus.modulus(),
			ciphertext_moduli,
			ciphertext_moduli_sizes,
			variance,
			ctx,
			op: op.map(Rc::new),
			delta: delta_poly,
			q_mod_t,
			scaler,
			plaintext: plaintext_modulus,
			mul_1_params,
			mul_2_params,
			matrix_reps_index_map,
		})
	}

	/// Generate ciphertext moduli with the specified sizes
	fn generate_moduli(
		ciphertext_moduli_sizes: &[usize],
		polynomial_degree: usize,
	) -> Result<Vec<u64>, BfvParametersBuilderError> {
		let mut moduli = vec![];
		for size in ciphertext_moduli_sizes {
			if *size > 62 || *size < 10 {
				return Err(BfvParametersBuilderError::ValidationError(
					"The moduli sizes must be between 10 and 62 bits.".to_string(),
				));
			}

			let mut upper_bound = 1 << size;
			loop {
				if let Some(prime) =
					generate_prime(*size, 2 * polynomial_degree as u64, upper_bound)
				{
					if !moduli.contains(&prime) {
						moduli.push(prime);
						break;
					} else {
						upper_bound = prime;
					}
				} else {
					return Err(BfvParametersBuilderError::ValidationError(
						"Could not generate enough ciphertext moduli to match the sizes provided"
							.to_string(),
					));
				}
			}
		}

		Ok(moduli)
	}
}

/// Multiplication parameters
#[derive(Debug, PartialEq, Eq, Default)]
pub(crate) struct MultiplicationParameters {
	pub(crate) extender_self: Scaler,
	pub(crate) extender_other: Scaler,
	pub(crate) down_scaler: Scaler,
}

impl MultiplicationParameters {
	fn new(
		from: &Rc<Context>,
		to: &Rc<Context>,
		up_self_factor: ScalingFactor,
		up_other_factor: ScalingFactor,
		down_factor: ScalingFactor,
	) -> Result<Self, String> {
		Ok(Self {
			extender_self: Scaler::new(from, to, up_self_factor)?,
			extender_other: Scaler::new(from, to, up_other_factor)?,
			down_scaler: Scaler::new(to, from, down_factor)?,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::{BfvParameters, BfvParametersBuilder};

	#[test]
	fn test_builder() {
		let params = BfvParametersBuilder::default().build();
		assert!(params.is_err_and(|e| e.to_string() == "`polynomial_degree` must be initialized"));

		let params = BfvParametersBuilder::default().polynomial_degree(7).build();
		assert!(params
			.is_err_and(|e| e.to_string()
				== "`polynomial_degree` must be a power of two larger or equal to 8"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1023)
			.build();
		assert!(params
			.is_err_and(|e| e.to_string()
				== "`polynomial_degree` must be a power of two larger or equal to 8"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1024)
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "`plaintext_modulus` must be initialized"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1024)
			.plaintext_modulus(0)
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "modulus should be between 2 and 2^62-1"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1024)
			.plaintext_modulus(2)
			.build();
		assert!(params
			.is_err_and(|e| e.to_string() == "One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1024)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![])
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(1024)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![1153])
			.ciphertext_moduli_sizes(vec![62])
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "One and only one of `ciphertext_moduli` or `ciphertext_moduli_sizes` must be specified"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![1])
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "modulus should be between 2 and 2^62-1"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![2])
			.build();
		assert!(params.is_err_and(|e| e.to_string() == "Impossible to construct a Ntt operator"));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![1153])
			.build();
		assert!(params.is_ok());

		let params = params.unwrap();
		assert_eq!(params.ciphertext_moduli, vec![1153]);
		assert_eq!(params.moduli(), vec![1153]);
		assert_eq!(params.plaintext_modulus, 2);
		assert_eq!(params.polynomial_degree, 8);
		assert_eq!(params.degree(), 8);
		assert_eq!(params.variance, 1);
		assert!(params.op.is_none());
	}

	#[test]
	fn test_default() {
		let params = BfvParameters::default(1);
		assert_eq!(params.ciphertext_moduli.len(), 1);

		let params = BfvParameters::default(2);
		assert_eq!(params.ciphertext_moduli.len(), 2);
	}

	#[test]
	fn test_ciphertext_moduli() {
		let params = BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(2)
			.ciphertext_moduli_sizes(vec![62, 62, 62, 61, 60, 11])
			.build();
		assert!(params.is_ok_and(|p| p.ciphertext_moduli
			== &[
				4611686018427387761,
				4611686018427387617,
				4611686018427387409,
				2305843009213693921,
				1152921504606846577,
				2017
			]));

		let params = BfvParametersBuilder::default()
			.polynomial_degree(8)
			.plaintext_modulus(2)
			.ciphertext_moduli(vec![
				4611686018427387761,
				4611686018427387617,
				4611686018427387409,
				2305843009213693921,
				1152921504606846577,
				2017,
			])
			.build();
		assert!(params.is_ok_and(|p| p.ciphertext_moduli_sizes == &[62, 62, 62, 61, 60, 11]));
	}
}
