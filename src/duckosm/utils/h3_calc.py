"""
H3 calculation utilities using efficient bitwise operations.
"""

def find_lca(cell1: int, cell2: int) -> int:
    """
    Find the Lowest Common Ancestor (LCA) of two H3 cells using fast bitwise operations.
    
    Args:
        cell1: First H3 cell (integer)
        cell2: Second H3 cell (integer)
        
    Returns:
        The H3 index of the LCA, or 0 if no common ancestor found.
    """
    if cell1 == 0 or cell2 == 0:
        return 0
    
    a = cell1
    b = cell2

    # 1. Extract resolutions (bits 52-55)
    res_a = (a >> 52) & 0xF
    res_b = (b >> 52) & 0xF
    common_res = min(res_a, res_b)

    # 2. Mask both to the coarsest input resolution
    # This zeroes out any 'extra' detail bits in the finer cell
    shift_alignment = 45 - (common_res * 3)
    alignment_mask = (0xFFFFFFFFFFFFFFFF << shift_alignment) & 0xFFFFFFFFFFFFFFFF
    
    a_aligned = a & alignment_mask
    b_aligned = b & alignment_mask

    # 3. XOR to find where they diverge
    # Mask out Resolution bits (52-55) to check only Base Cell + Index Digits
    RES_MASK = ~(0xF << 52) 
    diff = (a_aligned & RES_MASK) ^ (b_aligned & RES_MASK)

    if diff == 0:
        # One cell is a direct ancestor of the other
        return cell1 if res_a < res_b else cell2

    # 4. Find divergence resolution
    msb_diff = diff.bit_length()
    
    if msb_diff > 45: 
        return 0 # Different base cells -> No common ancestor

    diverge_res = (45 - msb_diff) // 3
    
    # 5. Construct the final Parent Index
    final_shift = 45 - (diverge_res * 3)
    
    lcp_int = (a & (0xFFFFFFFFFFFFFFFF << final_shift))
    lcp_int &= ~(0xF << 52)         # Clear resolution bits
    lcp_int |= (diverge_res << 52)  # Set new resolution bits
    
    # Set 'unused' child bits to 1s (padding)
    padding_mask = ~(0xFFFFFFFFFFFFFFFF << final_shift) & 0xFFFFFFFFFFFFFFFF
    lcp_int |= padding_mask

    return lcp_int

def get_resolution(cell: int) -> int:
    """
    Get the resolution of an H3 cell index (integer).
    
    Args:
        cell: H3 cell index
        
    Returns:
        Resolution (0-15) or -1 if invalid
    """
    if cell == 0:
        return -1
    return (cell >> 52) & 0xF
